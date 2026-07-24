import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import {
  WORKFLOW_SUB_AGENT_TOOL_NAMES,
  WORKFLOW_TODO_TOOL_NAMES
} from './toolClassification.js'

const projectRoot = new URL('../../../', import.meta.url)
const readProjectFile = relativePath =>
  readFileSync(new URL(relativePath, projectRoot), 'utf8')

const sourceSection = (source, startMarker, endMarker) => {
  const start = source.indexOf(startMarker)
  assert.notEqual(start, -1, `missing source marker: ${startMarker}`)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(end, -1, `missing source marker: ${endMarker}`)
  return source.slice(start, end)
}

const projectionRules = readProjectFile('src/composables/workflow/messageProjectionRules.js')
const structuredToolNameRule = sourceSection(
  projectionRules,
  'export const getStructuredWorkflowToolName',
  'export const isPendingApprovalEntryForTool'
)
assert.match(structuredToolNameRule, /metadata\.tool_name/)
assert.match(structuredToolNameRule, /metadata\.tool_call\?\.function\?\.name/)
assert.doesNotMatch(
  structuredToolNameRule,
  /metadata\.toolName|metadata\.title|metadata\.action|message\?*\.message|metadata\.content/
)

const completionRule = sourceSection(
  projectionRules,
  'export const isWorkflowCompletionMessage',
  'export const shouldRenderSubAgentCard'
)
assert.match(completionRule, /getStructuredWorkflowToolName\(message\) === 'complete_workflow'/)
assert.doesNotMatch(completionRule, /title|action|content|includes\(|startsWith\(/)

const toolStateMapper = readProjectFile('src/composables/workflow/useToolStateMapper.ts')
const metadataContract = sourceSection(
  toolStateMapper,
  'export interface MessageMetadata',
  '/** Tool call information */'
)
assert.match(metadataContract, /tool_name\?: string/)
assert.doesNotMatch(metadataContract, /toolName\?:/)

const extractToolName = sourceSection(
  toolStateMapper,
  'function extractToolName',
  'function extractArguments'
)
assert.match(extractToolName, /getStructuredWorkflowToolName\(message\)/)
assert.doesNotMatch(extractToolName, /title|action|message\.message|content/)

const workflowCore = readProjectFile('src/composables/workflow/useWorkflowCore.ts')
const approvePlan = sourceSection(workflowCore, 'const onApprovePlan', 'const onStop')
assert.doesNotMatch(approvePlan, /entry\?*\.action|includes\(['"]submit plan['"]\)/i)
assert.match(approvePlan, /isPendingApprovalEntryForTool\(entry, currentSessionId, 'submit_plan'\)/)

const approvalResolvedHandler = sourceSection(
  workflowCore,
  "} else if (payload.type === 'approval_resolved') {",
  "} else if (payload.type === 'tool_started') {"
)
assert.match(approvalResolvedHandler, /payload\.tool_name === 'submit_plan'/)
assert.match(approvalResolvedHandler, /resolvePendingTool\(sessionId, payload\.tool_call_id\)/)

const messageList = readProjectFile('src/components/workflow/WorkflowMessageList.vue')
assert.match(messageList, /:tool-name="getMessageToolName\(message\)"/)
assert.doesNotMatch(messageList, /:action="message\.metadata\?\.tool_name/)
assert.match(
  messageList,
  /isWorkflowMessagePendingApproval\(message, pendingApprovalIdSet\.value\)/
)

const approvalDialog = readProjectFile('src/components/workflow/ApprovalDialog.vue')
assert.match(approvalDialog, /toolName: String/)
assert.doesNotMatch(
  approvalDialog,
  /action: String|props\.action|normalizedAction|isFileChangePayload/
)

const classification = readProjectFile('src/composables/workflow/toolClassification.js')
assert.doesNotMatch(classification, /startsWith\s*\(/)

const rustConstants = readProjectFile('src-tauri/src/tools/constants.rs')
const rustToolName = constantName => {
  const match = rustConstants.match(
    new RegExp(`pub const ${constantName}: &str = "([^"]+)";`)
  )
  assert.ok(match, `missing Rust tool constant: ${constantName}`)
  return match[1]
}

assert.deepEqual(WORKFLOW_TODO_TOOL_NAMES, [
  rustToolName('TOOL_TODO_CREATE'),
  rustToolName('TOOL_TODO_LIST'),
  rustToolName('TOOL_TODO_UPDATE'),
  rustToolName('TOOL_TODO_GET')
])
assert.deepEqual(WORKFLOW_SUB_AGENT_TOOL_NAMES, [
  rustToolName('TOOL_SUB_AGENT_RUN'),
  rustToolName('TOOL_SUB_AGENT_OUTPUT'),
  rustToolName('TOOL_SUB_AGENT_STOP')
])

console.log('workflow constitution tests passed')
