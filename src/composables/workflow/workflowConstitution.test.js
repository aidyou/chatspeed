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
assert.match(workflowCore, /autoApprovePlan: autoApprovePlan\.value/)
assert.match(workflowCore, /'autoApprovePlan'/)
assert.match(
  workflowCore,
  /watch\(autoApprovePlan,[\s\S]*updateWorkflowConfig\('autoApprovePlan', !!newVal\)/
)
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
const toolGroupProjection = sourceSection(
  messageList,
  'const isToolWaitingApproval',
  'const visibleMessages'
)
assert.match(toolGroupProjection, /isWorkflowMcpTool\(toolName\)/)
assert.match(toolGroupProjection, /tool_group_kind: 'mcp_tools'/)
assert.match(toolGroupProjection, /icon: 'mcp'/)
assert.match(toolGroupProjection, /tool_group_kind: 'mixed_tools'/)
assert.match(
  toolGroupProjection,
  /while \(nextIndex < messages\.length && getCollapsibleToolGroupKind\(messages\[nextIndex\]\)\)/
)
assert.doesNotMatch(
  sourceSection(messageList, 'const isToolWaitingApproval', 'const isCollapsibleReadOnlyToolMessage'),
  /approval_submitted/,
  'approval-submitted and running tools must join their continuous tool group'
)

const toolIcons = readProjectFile('src/composables/workflow/toolIcons.ts')
assert.match(toolIcons, /isWorkflowMcpTool\(normalized\)\) return 'mcp'/)

const workflowView = readProjectFile('src/views/Workflow.vue')
const workflowInputArea = readProjectFile('src/components/workflow/WorkflowInputArea.vue')
const visibleAutoApprovalTools = sourceSection(
  workflowInputArea,
  'const workflowAvailableToolIds',
  'const allowedShellCommands'
)
assert.ok(
  visibleAutoApprovalTools.indexOf('props.currentWorkflow?.agentConfig?.availableTools') <
    visibleAutoApprovalTools.indexOf('props.selectedAgent?.availableTools'),
  'workflow tool capabilities must take precedence over a newer unsynchronized Agent definition'
)
assert.match(visibleAutoApprovalTools, /filter\(tool => availableSet\.has\(tool\)\)/)

const workflowConstitution = readProjectFile(
  'src-tauri/src/workflow/react/CONSTITUTION.md'
)
assert.match(workflowConstitution, /Auto-approved tools must be visible tools/)
assert.match(workflowConstitution, /auto-approved tool set must be a subset/)
assert.match(workflowConstitution, /Shell approval policy remains separate and cumulative/)
assert.match(workflowConstitution, /workflow-level shell `Allow` rules/)

assert.match(
  workflowInputArea,
  /v-if="showPlanningModeToggle && planningMode"[\s\S]*command="autoApprovePlan"/
)
assert.match(workflowInputArea, /emit\('toggle-auto-approve-plan'\)/)
assert.match(workflowInputArea, /:hide-on-click="false"/)
assert.match(
  workflowInputArea,
  /command === 'attachment'[\s\S]*quickActionsDropdownRef\.value\?\.handleClose\?\.\(\)[\s\S]*command === 'skillsConfig'[\s\S]*quickActionsDropdownRef\.value\?\.handleClose\?\.\(\)/
)
const workflowMessages = readProjectFile('src/composables/workflow/useWorkflowMessages.ts')
assert.doesNotMatch(
  workflowMessages,
  /visibleTaskGroupCount\.value\s*=\s*DEFAULT_VISIBLE_TASK_GROUPS/,
  'normal task transitions must preserve the history window explicitly expanded by the user'
)
const workflowSidebar = readProjectFile('src/components/workflow/WorkflowSidebar.vue')
const compactWorkflowProjection = sourceSection(
  workflowSidebar,
  'const compactActiveWorkflows',
  'const filteredAutomations'
)
assert.match(compactWorkflowProjection, /props\.workflows\.filter/)
assert.match(compactWorkflowProjection, /props\.workflows[\s\S]*\.slice\(0, 5\)/)
assert.doesNotMatch(
  compactWorkflowProjection,
  /filteredWorkflows\.value/,
  'compact mode must ignore expanded-mode search and directory filters'
)

const reactEngine = readProjectFile('src-tauri/src/workflow/react/engine.rs')
assert.doesNotMatch(reactEngine, /DEFAULT_MAX_STEPS|STEP BUDGET|max-step budget|self\.max_steps/)
assert.doesNotMatch(
  reactEngine,
  /Step budget:/,
  'runtime reminders must not pressure the model with a removed step limit'
)
assert.doesNotMatch(workflowCore, /confirmationWaiting|showConfirmationDialog/)
const workflowStore = readProjectFile('src/stores/workflow.js')
const clearContextEligibility = sourceSection(
  workflowStore,
  'const canClearContext = computed(() => {',
  'const canStop = computed(() => {'
)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.CANCELLED/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.COMPLETED/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.ERROR/)
assert.match(clearContextEligibility, /WORKFLOW_STATUSES\.FAILED/)
assert.match(
  clearContextEligibility,
  /workflowState !== WORKFLOW_STATUSES\.PENDING \|\| hasLiveSession\.value/
)
assert.ok(
  clearContextEligibility.indexOf('if (stoppedStates.includes(workflowState)) return true') <
    clearContextEligibility.indexOf('const executionState'),
  'terminal workflow status must override stale non-terminal execution context state'
)
const clearContextProjection = sourceSection(
  workflowView,
  'const onClearContextFrame = async () => {',
  '// Wrapper for skill select that properly handles send'
)
assert.match(
  clearContextProjection,
  /await workflowStore\.updateWorkflowStatus\([\s\S]*result\?\.state \|\| WORKFLOW_STATUSES\.PENDING[\s\S]*result\?\.waitReason/
)
assert.match(
  clearContextProjection,
  /workflowStore\.setHasLiveSession\(result\?\.hasLiveSession === true\)/
)
assert.ok(
  clearContextProjection.indexOf('await workflowStore.updateWorkflowStatus(') <
    clearContextProjection.indexOf('if (result?.noop)'),
  'clear-context noop recovery must reconcile backend lifecycle state before returning'
)
assert.ok(
  clearContextProjection.indexOf('visibleTaskGroupCount.value = 1') <
    clearContextProjection.indexOf('workflowStore.addMessage({'),
  'clear-context success must collapse expanded history before inserting the marker projection'
)

const approvalDialog = readProjectFile('src/components/workflow/ApprovalDialog.vue')
assert.match(approvalDialog, /toolName: String/)
assert.doesNotMatch(
  approvalDialog,
  /action: String|props\.action|normalizedAction|isFileChangePayload/
)

const deleteWorkflow = sourceSection(
  workflowCore,
  'const onDeleteWorkflow',
  'const createNewWorkflow'
)
assert.match(deleteWorkflow, /await invokeWrapper\('delete_workflow', \{ sessionId: id \}\)/)
assert.match(deleteWorkflow, /clearPendingApprovalEntries\(id\)/)
assert.match(deleteWorkflow, /backgroundStateListeners\.delete\(id\)/)
assert.match(deleteWorkflow, /setWorkflowDeleting\(id, true\)/)
assert.match(deleteWorkflow, /backendDeleteCompleted = true/)
assert.match(deleteWorkflow, /if \(!backendDeleteCompleted\)/)
assert.ok(
  deleteWorkflow.indexOf('setWorkflowDeleting(id, true)') <
    deleteWorkflow.indexOf("await invokeWrapper('delete_workflow'"),
  'deleting a workflow must block late approval events before invoking backend deletion'
)
assert.ok(
  deleteWorkflow.indexOf('clearPendingApprovalEntries(id)') <
    deleteWorkflow.indexOf('await workflowStore.loadWorkflows()'),
  'deleting a workflow must clear its background reminder cache before refreshing the sidebar'
)
assert.match(workflowCore, /if \(isWorkflowBeingDeleted\(sessionId\)\) return/)
assert.match(workflowView, /existingWorkflowIds\.has\(entry\?\.sessionId\)/)
assert.match(workflowView, /!isWorkflowBeingDeleted\(entry\?\.sessionId\)/)

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
