// Keep these exact names aligned with src-tauri/src/tools/constants.rs.
// Unknown tools intentionally remain unclassified until they are added here.
export const WORKFLOW_TODO_TOOL_NAMES = Object.freeze([
  'todo_create',
  'todo_list',
  'todo_update',
  'todo_get'
])

export const WORKFLOW_SUB_AGENT_TOOL_NAMES = Object.freeze([
  'sub_agent_run',
  'sub_agent_output',
  'sub_agent_stop'
])

const TODO_TOOL_NAMES = new Set(WORKFLOW_TODO_TOOL_NAMES)
const SUB_AGENT_TOOL_NAMES = new Set(WORKFLOW_SUB_AGENT_TOOL_NAMES)

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
