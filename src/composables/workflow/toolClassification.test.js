import assert from 'node:assert/strict'

import {
  getWorkflowToolFamily,
  isWorkflowMcpTool,
  isWorkflowTodoTool
} from './toolClassification.js'

for (const toolName of ['todo_create', 'todo_list', 'todo_update', 'todo_get']) {
  assert.equal(isWorkflowTodoTool(toolName), true, `${toolName} must be an exact Todo tool`)
  assert.equal(getWorkflowToolFamily(toolName), 'todo')
}

for (const toolName of ['sub_agent_run', 'sub_agent_output', 'sub_agent_stop']) {
  assert.equal(getWorkflowToolFamily(toolName), 'task')
}

for (const toolName of ['server__MCP__search', 'SERVER__mcp__WRITE', 'mcp_tool_load']) {
  assert.equal(isWorkflowMcpTool(toolName), true, `${toolName} must be classified as MCP`)
}

for (const toolName of ['mcp', 'mcp_search', 'server_mcp_search', 'load_mcp_tool']) {
  assert.equal(isWorkflowMcpTool(toolName), false, `${toolName} must not be inferred as MCP`)
}

for (const toolName of [
  'todo',
  'todo_archive',
  'todoist_import',
  'create_file',
  'task',
  'taskmaster',
  'sub_agent',
  'sub_agent_custom'
]) {
  assert.equal(isWorkflowTodoTool(toolName), false, `${toolName} must not be inferred as Todo`)
  assert.equal(getWorkflowToolFamily(toolName), null, `${toolName} must remain unclassified`)
}

console.log('toolClassification tests passed')
