const AUTO_EXECUTE_WORKFLOW_TOOLS = new Set([
  'ask_user',
  'complete_workflow_with_summary',
  'skill',
  'sub_agent_run',
  'sub_agent_output',
  'sub_agent_stop',
  'todo_create',
  'todo_get',
  'todo_list',
  'todo_update'
])

export function isAutoExecuteWorkflowTool(toolName?: string | null): boolean {
  return AUTO_EXECUTE_WORKFLOW_TOOLS.has(String(toolName || '').toLowerCase())
}
