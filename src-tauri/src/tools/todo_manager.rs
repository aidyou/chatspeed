use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::MainStore;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Helper to get and set todo list in DB
async fn get_db_todo_list(
    store: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
) -> Result<Vec<Value>, ToolError> {
    let store = store
        .read()
        .map_err(|e| ToolError::ExecutionFailed(format!("DB Lock error: {}", e)))?;
    let snapshot = store
        .get_workflow_snapshot(session_id)
        .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch workflow: {}", e)))?;

    let list_str = snapshot.workflow.todo_list.unwrap_or_else(|| "[]".into());
    serde_json::from_str(&list_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("Invalid todo JSON: {}", e)))
}

async fn save_db_todo_list(
    store: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    list: Vec<Value>,
) -> Result<(), ToolError> {
    let store = store
        .read()
        .map_err(|e| ToolError::ExecutionFailed(format!("DB Lock error: {}", e)))?;
    let list_str = serde_json::to_string(&list)
        .map_err(|e| ToolError::ExecutionFailed(format!("Serialize error: {}", e)))?;
    store
        .update_workflow_todo_list(session_id, &list_str)
        .map_err(|e| ToolError::ExecutionFailed(format!("DB Update error: {}", e)))
}

pub struct TodoCreateTool {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
}

#[async_trait]
impl ToolDefinition for TodoCreateTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TODO_CREATE
    }

    fn description(&self) -> &str {
        "Use this tool to create one or more structured tasks for your current session. This helps you track progress, organize complex tasks, and demonstrate thoroughness to the user.\n\
        It also helps the user understand the progress of the task and overall progress of their requests.\n\n\
        ## Capabilities\n\
        - **Bulk Creation**: You can pass an array of tasks in the `tasks` field to initialize a complete plan in one call.\n\
        - **Single Creation**: Alternatively, pass `subject` and `description` directly for a single task.\n\n\
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

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "subject": { "type": "string", "description": "Brief title of the task" },
                                "description": { "type": "string", "description": "Detailed description of the task" }
                            },
                            "required": ["subject", "description"]
                        },
                        "description": "An array of tasks to create at once"
                    },
                    "subject": { "type": "string", "description": "Brief title (if creating a single task)" },
                    "description": { "type": "string", "description": "Detailed description (if creating a single task)" }
                },
                "description": "Provide 'tasks' array for bulk creation, or 'subject'/'description' for a single item."
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let mut list = get_db_todo_list(&self.main_store, &self.session_id).await?;
        let mut created_ids = Vec::new();

        // Determine if we are creating bulk or single
        let tasks_to_create = if let Some(tasks) = params.get("tasks").and_then(|v| v.as_array()) {
            tasks.clone()
        } else if params.get("subject").is_some() {
            vec![params.clone()]
        } else {
            return Err(ToolError::InvalidParams(
                "Either 'tasks' array or 'subject' and 'description' must be provided".to_string(),
            ));
        };

        for task in tasks_to_create {
            let new_id = (list.len() + 1).to_string();
            let new_item = json!({
                "id": new_id,
                "subject": task["subject"].as_str().unwrap_or("Untitled"),
                "description": task["description"].as_str().unwrap_or(""),
                "status": "pending",
                "created_at": chrono::Local::now().to_rfc3339()
            });
            list.push(new_item);
            created_ids.push(new_id);
        }

        save_db_todo_list(&self.main_store, &self.session_id, list).await?;
        Ok(ToolCallResult::success(
            Some(format!(
                "Successfully created {} todo item(s). IDs: {}",
                created_ids.len(),
                created_ids.join(", ")
            )),
            None,
        ))
    }
}

pub struct TodoListTool {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
}

#[async_trait]
impl ToolDefinition for TodoListTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TODO_LIST
    }
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
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        let list = get_db_todo_list(&self.main_store, &self.session_id).await?;
        if list.is_empty() {
            return Ok(ToolCallResult::success(
                Some("Todo list is empty.".into()),
                None,
            ));
        }
        let output = list
            .iter()
            .map(|item| {
                format!(
                    "[{}] {} (ID: {})",
                    item["status"].as_str().unwrap_or("?"),
                    item["subject"].as_str().unwrap_or("Untitled"),
                    item["id"].as_str().unwrap_or("?")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolCallResult::success(Some(output), None))
    }
}

pub struct TodoUpdateTool {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
}

#[async_trait]
impl ToolDefinition for TodoUpdateTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TODO_UPDATE
    }
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
        Status progresses: `pending` → `in_progress` → `completed`.\n\
        Use `data_missing` when the required data could not be obtained but the task can be skipped.\n\
        Use `failed` when the task encountered an unrecoverable error.\n\
        Use `deleted` to permanently remove a task.\n\n\
        **Important**: When a task's data cannot be obtained after reasonable attempts, \
        mark it as `data_missing` rather than retrying indefinitely."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "deleted", "failed", "data_missing"] }
                },
                "required": ["todo_id", "status"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let todo_id = params["todo_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("todo_id required".into()))?;
        let status = params["status"]
            .as_str()
            .ok_or(ToolError::InvalidParams("status required".into()))?;

        let mut list = get_db_todo_list(&self.main_store, &self.session_id).await?;
        let mut found = false;

        if status == "deleted" {
            list.retain(|item| item["id"].as_str().map_or(false, |id| id != todo_id));
            found = true;
        } else {
            for item in list.iter_mut() {
                if item["id"].as_str().map_or(false, |id| id == todo_id) {
                    item["status"] = json!(status);
                    found = true;
                    break;
                }
            }
        }

        if !found {
            return Err(ToolError::ExecutionFailed(format!(
                "Todo item {} not found",
                todo_id
            )));
        }

        save_db_todo_list(&self.main_store, &self.session_id, list).await?;
        Ok(ToolCallResult::success(
            Some(format!("Updated todo item {} to {}", todo_id, status)),
            None,
        ))
    }
}

pub struct TodoGetTool {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
}

#[async_trait]
impl ToolDefinition for TodoGetTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TODO_GET
    }
    fn description(&self) -> &str {
        "Use this tool to retrieve a task by its ID from the todo list.\n\n\
        ## When to Use This Tool\n\
        - When you need the full description and context before starting work on a task\n\
        - To understand task dependencies\n\
        - After being assigned a task, to get complete requirements"
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "todo_id": { "type": "string" } },
                "required": ["todo_id"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let todo_id = params["todo_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("todo_id required".into()))?;
        let list = get_db_todo_list(&self.main_store, &self.session_id).await?;
        let item = list
            .iter()
            .find(|i| i["id"].as_str().map_or(false, |id| id == todo_id))
            .ok_or_else(|| ToolError::ExecutionFailed(format!("Todo {} not found", todo_id)))?;
        Ok(ToolCallResult::success(
            Some(serde_json::to_string_pretty(item).unwrap()),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Agent;
    use std::sync::RwLock;
    use tempfile::NamedTempFile;

    async fn setup_test_db() -> (Arc<RwLock<MainStore>>, String) {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path().to_path_buf();
        let store = MainStore::new(&db_path).unwrap();
        let store_arc = Arc::new(RwLock::new(store));
        let session_id = "test_session".to_string();
        let agent_id = "test_agent".to_string();

        {
            let store_guard = store_arc.read().unwrap();
            let agent = Agent::new(
                agent_id.clone(),
                "Test Agent".to_string(),
                None,
                "System prompt".to_string(),
                None,
                None,
                None,
                None, // models
                None, // shell_policy
                None, // allowed_paths
                None, // final_audit
                None, // approval_level
                None, // max_contexts
            );
            store_guard.add_agent(&agent).unwrap();
            // Pass None for agent_config
            store_guard
                .create_workflow(&session_id, "Test prompt", &agent_id, None)
                .unwrap();
        }

        (store_arc, session_id)
    }

    #[tokio::test]
    async fn test_todo_workflow() {
        let (store, session_id) = setup_test_db().await;

        // 1. Test Create
        let create_tool = TodoCreateTool {
            session_id: session_id.clone(),
            main_store: store.clone(),
        };
        let res = create_tool
            .call(json!({
                "subject": "Task 1",
                "description": "First task description"
            }))
            .await
            .unwrap();
        assert!(res.content.unwrap().contains("ID: 1"));

        // 2. Test List
        let list_tool = TodoListTool {
            session_id: session_id.clone(),
            main_store: store.clone(),
        };
        let res = list_tool.call(json!({})).await.unwrap();
        assert!(res.content.unwrap().contains("[pending] Task 1 (ID: 1)"));

        // 3. Test Update
        let update_tool = TodoUpdateTool {
            session_id: session_id.clone(),
            main_store: store.clone(),
        };
        let res = update_tool
            .call(json!({
                "todo_id": "1",
                "status": "in_progress"
            }))
            .await
            .unwrap();
        assert!(res
            .content
            .unwrap()
            .contains("Updated todo item 1 to in_progress"));

        // Verify update in list
        let res = list_tool.call(json!({})).await.unwrap();
        assert!(res
            .content
            .unwrap()
            .contains("[in_progress] Task 1 (ID: 1)"));

        // 4. Test Get
        let get_tool = TodoGetTool {
            session_id: session_id.clone(),
            main_store: store.clone(),
        };
        let res = get_tool.call(json!({ "todo_id": "1" })).await.unwrap();
        let val: Value = serde_json::from_str(&res.content.unwrap()).unwrap();
        assert_eq!(val["subject"], "Task 1");
        assert_eq!(val["status"], "in_progress");

        // 5. Test Delete
        let res = update_tool
            .call(json!({
                "todo_id": "1",
                "status": "deleted"
            }))
            .await
            .unwrap();
        assert!(res
            .content
            .unwrap()
            .contains("Updated todo item 1 to deleted"));

        // Verify empty list
        let res = list_tool.call(json!({})).await.unwrap();
        assert!(res.content.unwrap().contains("Todo list is empty."));
    }
}
