import assert from 'node:assert/strict'

import { getWorkflowToolFamily, isWorkflowTodoTool } from './toolClassification.js'

for (const toolName of ['todo_create', 'todo_list', 'todo_update', 'todo_get']) {
  assert.equal(isWorkflowTodoTool(toolName), true, `${toolName} must be an exact Todo tool`)
  assert.equal(getWorkflowToolFamily(toolName), 'todo')
}

for (const toolName of ['sub_agent_run', 'sub_agent_output', 'sub_agent_stop']) {
  assert.equal(getWorkflowToolFamily(toolName), 'task')
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
