const AUTO_EXECUTE_WORKFLOW_TOOLS = new Set([
  'ask_user',
  'finish_task',
  'skill',
  'task',
  'task_output',
  'task_stop',
  'todo_create',
  'todo_get',
  'todo_list',
  'todo_update'
])

export function isAutoExecuteWorkflowTool(toolName?: string | null): boolean {
  return AUTO_EXECUTE_WORKFLOW_TOOLS.has(String(toolName || '').toLowerCase())
}
