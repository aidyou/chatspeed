// Keep these exact names aligned with src-tauri/src/tools/constants.rs.
// Unknown tools intentionally remain unclassified until they are added here.
const TODO_TOOL_NAMES = new Set([
  'todo_create',
  'todo_list',
  'todo_update',
  'todo_get'
])

const SUB_AGENT_TOOL_NAMES = new Set([
  'sub_agent_run',
  'sub_agent_output',
  'sub_agent_stop'
])

const normalizeToolName = toolName =>
  String(toolName || '')
    .trim()
    .toLowerCase()

export const isWorkflowTodoTool = toolName => TODO_TOOL_NAMES.has(normalizeToolName(toolName))

export const getWorkflowToolFamily = toolName => {
  const normalized = normalizeToolName(toolName)
  if (TODO_TOOL_NAMES.has(normalized)) return 'todo'
  if (SUB_AGENT_TOOL_NAMES.has(normalized)) return 'task'
  return null
}
