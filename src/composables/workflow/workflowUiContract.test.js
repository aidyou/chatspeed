import assert from 'node:assert/strict'

import {
  isWorkflowCompletionMessage,
  isWorkflowMessagePendingApproval
} from './messageProjectionRules.js'
import { deriveInlinePendingApprovals } from '../../stores/workflowApprovalRecovery.js'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const sessionId = 'session-approval-contract'
const toolCallId = 'tool_571ae521'
const command =
  'sqlite3 workflow.db "SELECT InvalidFinishSummary, FinishTask FROM workflow_messages"'
const approvalWaitingStatuses = ['awaiting_approval', 'awaiting_auto_approval']
const executionContext = {
  wait_reason: 'approval',
  pending_tools: [
    {
      tool_call_id: toolCallId,
      tool_name: 'bash',
      arguments: { command },
      details: { command },
      display_type: 'text'
    }
  ]
}
const pendingMessage = {
  id: 63713,
  sessionId,
  role: 'tool',
  message: command,
  metadata: {
    tool_call_id: toolCallId,
    tool_name: 'bash',
    tool_call: {
      id: toolCallId,
      function: {
        name: 'bash',
        arguments: { command }
      }
    },
    details: { command },
    approval_status: 'pending',
    execution_status: 'pending_approval'
  },
  toolDisplay: {
    action: command,
    displayType: 'text'
  }
}

const pendingApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext,
  messages: [pendingMessage],
  approvalWaitingStatuses
})
const pendingIds = pendingApprovals.map(approval => approval.toolCallId)

assert.equal(pendingApprovals.length, 1, 'the Bash request must produce one pending approval')
assert.equal(pendingApprovals[0].toolName, 'bash', 'the pending item must retain Bash identity')
assert.equal(
  isWorkflowCompletionMessage(pendingMessage),
  false,
  'command text containing completion markers must not select the completion presentation'
)
assert.equal(
  isWorkflowMessagePendingApproval(pendingMessage, pendingIds),
  true,
  'the pending Bash message must render its inline approval controls'
)

const resolvedMessage = {
  ...pendingMessage,
  id: 63714,
  metadata: {
    ...pendingMessage.metadata,
    approval_status: 'approved',
    execution_status: 'completed'
  }
}
const resolvedApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext,
  messages: [pendingMessage, resolvedMessage],
  approvalWaitingStatuses
})

assert.equal(resolvedApprovals.length, 0, 'the latest resolved state must remove the pending item')
assert.equal(
  isWorkflowMessagePendingApproval(pendingMessage, resolvedApprovals.map(approval => approval.toolCallId)),
  false,
  'a stale pending transcript row must not keep approval controls visible after resolution'
)

const submittedApprovalIds = new Set([toolCallId])
const submittedApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: sessionId,
  workflowTitle: 'Approval UI contract',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext: {
    ...executionContext,
    pending_tools: [
      ...executionContext.pending_tools,
      { tool_call_id: 'tool_still_pending', tool_name: 'write_file' }
    ]
  },
  messages: [pendingMessage],
  submittedToolIds: submittedApprovalIds,
  approvalWaitingStatuses
})
assert.deepEqual(
  submittedApprovals.map(approval => approval.toolCallId),
  ['tool_still_pending'],
  'locally submitted child approvals must leave only the remaining structured pending tools'
)

console.log('workflow UI contract tests passed')

test('authorized root drag sorting stays on the existing structured allowed-path update path', async () => {
  const [fileTree, sidebar, workflowView, workflowPaths, workflowStore, workflowCommand, pathGuard, engine] =
    await Promise.all([
      readFile('src/components/workflow/FileTree.vue', 'utf8'),
      readFile('src/components/workflow/WorkflowSidebar.vue', 'utf8'),
      readFile('src/views/Workflow.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowPaths.ts', 'utf8'),
      readFile('src/stores/workflow.js', 'utf8'),
      readFile('src-tauri/src/commands/workflow.rs', 'utf8'),
      readFile('src-tauri/src/workflow/react/security.rs', 'utf8'),
      readFile('src-tauri/src/workflow/react/engine.rs', 'utf8')
    ])

  assert.match(fileTree, /import \{ Sortable \} from 'sortablejs-vue3'/)
  assert.match(fileTree, /<Sortable[\s\S]*?:list="rootItems"[\s\S]*?@update="onRootSort"/)
  assert.match(fileTree, /const rootItems = ref\(\[\]\)/)
  assert.match(fileTree, /filter: '\.root-actions'/)
  assert.match(fileTree, /rootItems\.value = newPaths\.map\(path => \(\{ path \}\)\)/)
  assert.match(fileTree, /emit\('reorderPaths', reorderedPaths\)/)
  assert.doesNotMatch(fileTree, /onRootDrop|onRootDragStart|:draggable=/)
  assert.match(sidebar, /@reorder-paths="\$emit\('reorder-paths-from-tree', \$event\)"/)
  assert.match(workflowView, /@reorder-paths-from-tree="onReorderPathsFromTree"/)
  assert.match(workflowPaths, /const onReorderPathsFromTree = async paths => \{[\s\S]*?updateWorkflowAllowedPaths\(currentWorkflowId\.value, paths\)/)
  assert.match(workflowPaths, /updateAutomationAllowedPaths\(selectedAutomation\.value\.id, paths\)/)
  assert.match(workflowStore, /invokeWrapper\('update_workflow_allowed_paths', \{[\s\S]*?allowedPaths: allowedPaths/)
  assert.match(workflowCommand, /"type": "update_allowed_paths",[\s\S]*?"paths": runtime_paths/)
  assert.match(pathGuard, /self\.primary_root = self\.workspace_roots\.first\(\)/)
  assert.match(engine, /guard\.update_allowed_roots\(paths\.clone\(\)\);[\s\S]*?self\.planning_root = Self::planning_root_for_allowed_paths\(&paths\)/)
})

test('empty new workflows require an authorized directory before accepting input', async () => {
  const [inputArea, workflowView, workflowPaths] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowPaths.ts', 'utf8')
  ])

  assert.match(inputArea, /:readonly="requiresAuthorizedPathSelection"/)
  assert.match(
    inputArea,
    /Boolean\(props\.currentWorkflowId && props\.currentWorkflow\)[\s\S]*?!props\.currentWorkflow\.userQuery\?\.trim\(\)[\s\S]*?props\.currentPaths\.length === 0/
  )
  assert.match(inputArea, /@click="handleInputClick"/)
  assert.match(
    inputArea,
    /const handleInputClick = async \(\) => \{[\s\S]*?await props\.onAddAuthorizedPath\?\.\(\)[\s\S]*?await nextTick\(\)[\s\S]*?!requiresAuthorizedPathSelection\.value[\s\S]*?inputRef\.value\?\.focus\(\)/
  )
  assert.match(workflowView, /:current-paths="currentPaths"[\s\S]*?:on-add-authorized-path="onAddPath"/)
  assert.match(workflowView, /displayAllowedPath,[\s\S]*?onAddPath,[\s\S]*?onAddPathFromTree/)
  assert.match(
    workflowPaths,
    /const onAddPath = async \(\) => \{[\s\S]*?await open\([\s\S]*?directory: true[\s\S]*?pendingPaths\.value\.push\(selected\)/
  )
})

test('child session panes preserve a confirm received while snapshot hydration is in flight', async () => {
  const [workflowView, messageList, sessionPane, sessionMessages, workflowCore, engine, workflowCommand, workflowDb] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowSessionMessages.ts', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
    readFile('src-tauri/src/workflow/react/engine.rs', 'utf8'),
    readFile('src-tauri/src/commands/workflow.rs', 'utf8'),
    readFile('src-tauri/src/db/workflow.rs', 'utf8')
  ])

  assert.match(messageList, /<cs name="fullscreen-off"\s*\/>/)
  assert.match(messageList, /sub-agent-card__status[\s\S]*?sub-agent-card__open-button/)
  assert.match(messageList, /sub-agent-card__title-wrap[\s\S]*?sub-agent-card__meta/)
  assert.match(messageList, /@click\.stop="\$emit\('open-sub-agent', message\.subAgentCard\.taskId\)"/)
  assert.match(workflowView, /v-if="activeSubAgentSessionId && activeSubAgentParentSessionId === currentWorkflowId"/)
  assert.match(workflowView, /const openSubAgentMessagePane = sessionId => \{[\s\S]*?activeSubAgentParentSessionId\.value = parentSessionId/)
  assert.match(workflowView, /watch\([\s\S]*?\(\) => workflowStore\.currentWorkflowId,[\s\S]*?currentSessionId !== previousSessionId[\s\S]*?closeSubAgentMessagePane\(\)/)
  assert.match(workflowView, /await selectWorkflow\(navigationSessionId\)[\s\S]*?if \(entry\?\.subAgentId\) openSubAgentMessagePane\(entry\.subAgentId\)/)
  assert.match(workflowView, /\.workflow-chat-pane \{[\s\S]*?position: relative;/)
  assert.match(workflowView, /\.sub-agent-message-overlay \{[\s\S]*?position: absolute;[\s\S]*?inset: 0;/)
  assert.match(sessionPane, /:messages="messageProjection\.enhancedMessages\.value"/, 'the message list must receive the projected message array, not its computed ref')
  assert.match(sessionPane, /:hidden-earlier-message-count="messageProjection\.hiddenEarlierMessageCount\.value"/, 'the message-list history count must receive its computed value')
  assert.match(sessionPane, /:last-assistant-message="messageProjection\.lastAssistantMessage\.value"/, 'the message list must receive the resolved last assistant message')
  assert.match(sessionPane, /:approval-loading="approval\.approvalLoading\.value"/, 'the message list must receive the approval loading boolean, not its ref')
  assert.match(sessionPane, /:active-approval-id="approval\.activeApprovalId\.value"/, 'the message list must receive the active approval ID, not its ref')
  assert.match(sessionPane, /:is-approval-submitting="resolvedIsApprovalSubmitting"/, 'child panes must use their session-local approval submission state')
  assert.match(sessionPane, /<cs name="fullscreen"\s*\/>/)
  assert.match(sessionPane, /class="workflow-session-message-pane__status"[\s\S]*?:class="childStatusClass"/)
  assert.match(sessionPane, /workflow\.subAgent\.statusCompleted/)
  assert.match(sessionPane, /workflow\.subAgent\.statusCancelled/)
  assert.match(sessionPane, /agentStore\.agents\.find/)
  assert.doesNotMatch(sessionPane, /WorkflowInputArea/)
  assert.match(sessionMessages, /hydrateWorkflowSession\(/, 'child hydration must install its event buffer before listener registration')
  assert.match(sessionMessages, /registerListener: handleEvent => listen\(`workflow:\/\/event\/\$\{targetSessionId\}`/, 'child hydration must listen to its own channel')
  assert.match(sessionMessages, /fetchSnapshot: \(\) => invokeWrapper\('get_workflow_snapshot', \{ sessionId: targetSessionId \}\)/, 'child hydration must use the canonical snapshot')
  assert.match(sessionMessages, /applyEvent,[\s\S]*?isCurrent: \(\) => revision === loadRevision\.value && targetSessionId === sessionId\.value,[\s\S]*?onListenerRegistered: stop =>/, 'the listener must be retained from registration through snapshot replay for the active child session')
  assert.match(sessionPane, /const resolvedIsApprovalSubmitting = \(sessionId, toolCallId\) =>[\s\S]*?childSession\.isApprovalSubmitting\(toolCallId\)/)
  assert.match(sessionMessages, /const approvalSubmissions = ref\(new Set\(\)\)/)
  assert.match(sessionMessages, /submittedToolIds: approvalSubmissions\.value/)
  assert.match(sessionMessages, /const markApprovalSubmitted = toolCallId =>/)
  assert.match(sessionMessages, /const clearApprovalSubmission = toolCallId =>/)
  assert.match(sessionMessages, /const isApprovalSubmitting = toolCallId =>[\s\S]*?approvalSubmissions\.value\.has/)
  assert.match(sessionMessages, /payload\.type === 'approval_resolved'[\s\S]*?clearApprovalSubmission\(toolCallId\)/)
  assert.match(sessionMessages, /payload\.type === 'tool_started'[\s\S]*?clearApprovalSubmission\(toolCallId\)/)
  assert.match(sessionPane, /markApprovalSubmitted:[\s\S]*?childSession\.markApprovalSubmitted\(toolCallId\)[\s\S]*?clearApprovalSubmission:[\s\S]*?childSession\.clearApprovalSubmission\(toolCallId\)[\s\S]*?isApprovalSubmitted:[\s\S]*?childSession\.isApprovalSubmitting\(toolCallId\)/)
  assert.match(sessionPane, /onApproveAction\(toolCallId, props\.sessionId\)/, 'child approvals must still target the child session')
  assert.match(
    engine,
    /Active execution must consume only signals it owns[\s\S]*?sub-agent approval that arrives while a previous tool runs[\s\S]*?per-session FIFO stash until its compatible wait state is entered/,
    'active child execution must preserve approvals until AwaitingApproval owns them'
  )
  assert.match(
    engine,
    /async fn check_stop_signal[\s\S]*?signal_deferred_non_waiting[\s\S]*?stash_runtime_signal\(&self\.session_id, s\)/,
    'the active signal drain must defer typed wait signals instead of consuming child approvals'
  )
  assert.match(
    engine,
    /let signal_str = if let Some\(signal\)\s*=\s*take_stashed_runtime_signal\(&self\.session_id\)[^]*?Signal receiver returned None while waiting[^]*?Signal channel closed/,
    'AwaitingApproval must consume deferred signals before reading the live channel'
  )
  assert.match(sessionMessages, /workflow:\/\/event\/\$\{targetSessionId\}/, 'child pane must listen to its own channel')
  assert.doesNotMatch(sessionMessages, /selectWorkflow\(/, 'child pane must not replace the root selection')
  assert.match(workflowCore, /targetSessionId: payload\.sub_agent_id,[\s\S]*?navigationSessionId: payload\.parent_session_id/)
  assert.match(
    messageList,
    /const inlineBulkApprovalCount = computed\(\(\) => props\.pendingCount\)/,
    'the bulk approval count must follow the canonical pending ID count instead of rendered message timing'
  )
  assert.match(
    messageList,
    /const pendingIdSet = new Set\(pendingIds\)[\s\S]*?pendingIdSet\.has\(toolCallId\)/,
    'bulk approval ordering must be constrained by the canonical pending ID set'
  )
  assert.match(
    workflowCore,
    /const handleSubAgentApprovalRequested = payload =>[\s\S]*?playApprovalNotificationSound\(\)/,
    'sub-agent approval requests must use the shared reminder and notification path'
  )
  assert.match(
    workflowCore,
    /if \(payload\.type === 'sub_agent_approval_requested'\) \{\s*handleSubAgentApprovalRequested\(payload\)/,
    'active workflow listeners must process bridged sub-agent approvals'
  )
  assert.match(
    workflowCore,
    /workflowStore\.clearApprovalSubmission\(sessionId, payload\.tool_call_id\)/,
    'background approval resolution must clear local submission state'
  )
  assert.match(
    workflowCore,
    /if \(workflow\.id === activeSessionId\) return false/,
    'the active workflow must have one authoritative event listener'
  )
  assert.match(sessionPane, /:pending-count="resolvedPendingApprovalIds\.length"/)
  assert.match(
    engine,
    /SubAgentApprovalRequested \{[\s\S]*?arguments: arguments\.clone\(\),[\s\S]*?details: details\.clone\(\)/,
    'sub-agent approval bridges must preserve structured approval payloads'
  )
  assert.match(workflowCore, /const clearPendingApprovalEntriesBySubAgent = subAgentId =>/)
  assert.match(workflowCore, /nextEntries\[key\]\?\.subAgentId !== normalizedSubAgentId/)
  assert.match(workflowCore, /sub_agent_progress[\s\S]*?clearPendingApprovalEntriesBySubAgent\(payload\.sub_agent_id\)/)
  assert.match(workflowCore, /\['completed', 'failed', 'cancelled', 'interrupted', 'error'\]/)
  assert.match(engine, /GatewayPayload::SubAgentApprovalRequested/)
  assert.match(engine, /GatewayPayload::SubAgentApprovalResolved/)
  assert.match(
    engine,
    /WorkflowState::Completed\s*\| WorkflowState::Cancelled\s*\| WorkflowState::Error/,
    'terminal child workflows must clear their authoritative pending approvals'
  )
  assert.match(
    workflowCore,
    /const activeParentWorkflowIds = new Set\([\s\S]*?!TERMINAL_STATUSES\.includes\(status\)/,
    'the frontend must skip restoration when every parent workflow is terminal'
  )
  assert.match(workflowCore, /if \(!activeParentWorkflowIds\.size\) return/)
  assert.match(workflowCore, /if \(!activeParentWorkflowIds\.has\(parentSessionId\)\) continue/)
  assert.match(workflowCommand, /list_child_workflows_with_pending_approvals\(\)/)
  assert.match(
    workflowDb,
    /status IN \('awaiting_approval', 'awaiting_auto_approval'\)/,
    'the backend query must only load child workflows currently waiting for approval'
  )
  assert.match(
    workflowDb,
    /parent\.status NOT IN \('completed', 'failed', 'error', 'cancelled'\)/,
    'the backend query must exclude children of terminal parents before loading snapshots'
  )
})

test('MCP tool calls show their arguments and format only valid JSON results', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /const isMcpToolMessage = message =>[\s\S]*?isWorkflowMcpTool\(getMessageToolName\(message\), getMessageToolCategory\(message\)\)/
  )
  assert.match(messageList, /class="mcp-tool-arguments"/)
  assert.match(messageList, /getFormattedMcpToolArguments\(tool\)/)
  assert.match(messageList, /getFormattedMcpToolArguments\(message\)/)
  assert.match(messageList, /JSON\.stringify\(JSON\.parse\(value\), null, 2\)/)
  assert.match(messageList, /catch \{\s*return value\s*\}/)
  assert.match(messageList, /getMcpToolContentForDisplay\(tool\)/)
  assert.match(messageList, /getMcpToolContentForDisplay\(message\)/)
})

test('ask-user responses stay hidden from the transcript and render on their source tool card', async () => {
  const [workflowView, workflowCore, workflowMessages, messageList, workflowEngine, workflowStore] =
    await Promise.all([
      readFile('src/views/Workflow.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
      readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
      readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
      readFile('src-tauri/src/workflow/react/engine.rs', 'utf8'),
      readFile('src/stores/workflow.js', 'utf8')
    ])

  assert.match(
    workflowView,
    /const submitAskUserResponse = async response => \{[\s\S]*?coreOnSendMessage\(content, \{[\s\S]*?ui_visibility: 'hide',[\s\S]*?ask_user_response: true,[\s\S]*?requested_tool_call_id/,
    'ask-user answers must remain hidden and carry an association hint'
  )
  assert.match(
    workflowCore,
    /if \(options\.metadata\) \{\s*signalPayload\.metadata = options\.metadata/,
    'hidden-message metadata must continue through the user-message signal sent to the runtime'
  )
  const awaitingUserMetadataWrites = workflowEngine.match(
    /WorkflowSignal::UserMessage \{\s*content, metadata, \.\.\s*\}[\s\S]*?canonicalize_ask_user_response_metadata\(metadata\)[\s\S]*?add_message_and_notify_internal\([\s\S]*?metadata,\s*\)/g
  )
  assert.equal(
    awaitingUserMetadataWrites?.length,
    2,
    'both awaiting-user signal branches must persist backend-canonicalized answer metadata'
  )
  assert.match(
    workflowMessages,
    /const askUserResponsesByToolCallId = computed\(\(\) => \{[\s\S]*?getMessageToolCallId\(message\)[\s\S]*?responses\.set\(toolCallId, message\.message \|\| ''\)/,
    'the hidden answer must be indexed by its canonical tool_call_id'
  )
  assert.match(
    workflowMessages,
    /if \(!getMessageToolCallId\(message\)\) \{\s*askUserResponsesBySourceOrder\.set\(pendingAskUserSourceOrder, message\.message\)/,
    'positional ask-user response matching must be limited to legacy rows without tool_call_id'
  )
  assert.match(
    workflowMessages,
    /askUserResponse: resolveAskUserResponse\([\s\S]*?askUserResponsesByToolCallId\.value,[\s\S]*?askUserResponsesBySourceOrder/,
    'the source ask_user tool must receive its response through the ID-first projection rule'
  )
  assert.match(
    workflowMessages,
    /if \(isApprovedPlanAnchor\(message\) \|\| isHiddenAskUserResponse\(message\)\) return false/,
    'the hidden answer must stay out of the transcript after its association is derived'
  )
  assert.match(
    workflowMessages,
    /askUserResponse: message\?\.askUserResponse \|\| ''/,
    'ask-user response attachment must invalidate the enhanced-message cache'
  )
  assert.match(
    messageList,
    /@focus="selectAskUserOtherIfUnset\(message, group\.title\)"/,
    'focusing the supplemental input must select Other when the user has not selected an option'
  )
  assert.match(
    messageList,
    /const selectAskUserOtherIfUnset = \(message, title\) => \{[\s\S]*?OTHER_ASK_USER_VALUE/,
    'the focus handler must select the fixed Other value only when unset'
  )
  assert.match(
    messageList,
    /const getToolRenderStatusClass = message => \{[\s\S]*?isToolGroupItemRunning\(message\)/,
    'standalone and grouped tools must share the running/error/success class mapping'
  )
  assert.match(
    workflowView,
    /const batchApprovalSessionId = ref\(''\)[\s\S]*?if \(!sessionId \|\| batchApprovalSessionId\.value\) return/,
    'bulk approval must be locked per active workflow session'
  )
  assert.match(
    messageList,
    /props\.isBatchApprovalSubmitting[\s\S]*?orderedToolCallIds: pendingToolCallIds/,
    'all inline approval cards must disable duplicate batch submissions while the batch is active'
  )
  assert.match(
    workflowStore,
    /invokeWrapper\('get_earlier_workflow_message_page'/,
    'history pagination must use the regular message page endpoint'
  )
  assert.doesNotMatch(
    workflowStore,
    /hasMoreInCurrentTask|hiddenCompletedTaskCount|get_earlier_workflow_messages/,
    'task-segment pagination state and endpoint must not remain in the frontend store'
  )
  assert.doesNotMatch(
    workflowView,
    /withinCurrentTask|loadEarlierTaskGroup|revealEarlierTaskGroup/,
    'the workflow view must request ordinary message pages'
  )
  assert.match(
    messageList,
    /<template v-if="getAskUserResponseItems\(message\)\.length > 0">[\s\S]*?v-for="group in getChoiceGroups\(message\)"[\s\S]*?choice-options--readonly[\s\S]*?isAskUserOptionSelected[\s\S]*?isAskUserAnswerOther[\s\S]*?getAskUserAnswerSupplement/,
    'an answered ask_user card must retain original choices, mark the selected option, and show supplied text'
  )
  assert.match(
    messageList,
    /const getAskUserResponseForGroup = \(message, title\) =>[\s\S]*?item\?\.title === title/,
    'answered choices must resolve their associated response by question title from the canonical tool response'
  )
  assert.match(
    messageList,
    /const OTHER_ASK_USER_VALUE = '__other__'/,
    'the fixed Other choice must have a distinct non-user option value'
  )
  assert.match(
    messageList,
    /selection === OTHER_ASK_USER_VALUE[\s\S]*?validationOtherRequired/,
    'Other must require supplemental text before submission'
  )
  assert.match(
    messageList,
    /choice: supplement \? `\$\{selection\}\\n\$\{supplement\}` : selection/,
    'standard choices must include an optional supplemental line in their submitted result'
  )
})

test('workflow quick actions expose manual compression through the slash-command handler', async () => {
  const [inputArea, workflowView, signalTypes] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/signalTypes.ts', 'utf8')
  ])

  assert.match(
    inputArea,
    /<el-dropdown-item command="manualCompress">[\s\S]*?<cs name="compress"[\s\S]*?manualCompressShort/,
    'the quick-actions menu must place the manual compression action beside attachments'
  )
  assert.match(inputArea, /emit\('trigger-manual-compress'\)/)
  assert.match(workflowView, /@trigger-manual-compress="triggerManualCompression"/)
  assert.match(workflowView, /type: SIGNAL_TYPES\.MANUAL_COMPRESS/)
  assert.match(signalTypes, /MANUAL_COMPRESS: 'manual_compress'/)
})

test('workflow slash commands manage authorized directories', async () => {
  const [inputArea, workflowView] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8')
  ])

  assert.match(workflowView, /name: 'add-dir'[\s\S]*?commandDirAddDesc[\s\S]*?requiresArgument: true/)
  assert.match(workflowView, /name: 'remove-dir'[\s\S]*?commandDirRemoveDesc/)
  assert.match(
    workflowView,
    /commandName === 'add-dir'[\s\S]*?unwrapDirectoryCommandPath\(commandArgument\)[\s\S]*?addAuthorizedPathFromCommand\(path\)/,
    'add-dir must pass its path argument through the existing authorized-path update flow'
  )
  assert.match(
    workflowView,
    /import \{ homeDir \} from '@tauri-apps\/api\/path'[\s\S]*?const expandDirectoryCommandPath = async value =>[\s\S]*?homeDir\(\)[\s\S]*?return `\$\{home\}\$\{path\.slice\(1\)\}`/,
    'add-dir must expand a leading tilde through the Tauri home directory'
  )
  assert.match(
    workflowView,
    /commandName === 'remove-dir'[\s\S]*?openDirectoryRemovalPanel\?\.\(\)/,
    'remove-dir must open the directory removal panel'
  )
  assert.match(
    inputArea,
    /directoryRemovalPanelVisible[\s\S]*?v-for="path in currentPaths"[\s\S]*?directory-command-remove[\s\S]*?removeDirectoryPath\(path\)/,
    'the removal panel must list current paths with an action button for each path'
  )
  assert.match(inputArea, /const openDirectoryRemovalPanel = \(\) =>/)
  assert.match(inputArea, /defineExpose\(\{[\s\S]*?openDirectoryRemovalPanel/)

  const workflowInput = await readFile('src/composables/workflow/useWorkflowInput.ts', 'utf8')
  assert.match(
    workflowInput,
    /skill\.type === 'command' && skill\.requiresArgument === true[\s\S]*?inputMessage\.value = '\/' \+ skill\.name \+ ' '/,
    'argument-taking commands must be inserted into the composer before execution'
  )
  assert.match(
    workflowInput,
    /skill\.type === 'command' && skill\.requiresArgument === true[\s\S]*?return\n\s*}\n\n\s*if \(skill\.type === 'command' && typeof onBuiltinCommandSelect === 'function'\)[\s\S]*?onBuiltinCommandSelect\(skill\)/,
    'argument-taking commands must return before the ordinary command callback'
  )
})

test('auto-compression starts disabled until explicitly enabled', async () => {
  const [workflowView, workflowCore] = await Promise.all([
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  ])

  assert.match(workflowView, /const autoCompressEnabled = ref\(false\)/)
  assert.match(workflowView, /autoCompressEnabled\.value = agentConfig\?\.autoCompress \?\? false/)
  assert.match(workflowCore, /autoCompressEnabled\.value = config\.autoCompress \?\? false/)
})

test('execution style popover uses a DOM reference and preserves Agent-scoped choices', async () => {
  const [inputArea, workflowView, workflowCore] = await Promise.all([
    readFile('src/components/workflow/WorkflowInputArea.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')
  ])

  assert.match(
    inputArea,
    /<el-dropdown-item command="skillsConfig"[\s\S]*?<el-popover[\s\S]*?v-model:visible="autoApprovedPopoverVisible"[\s\S]*?placement="right-start"[\s\S]*?<span class="tool-config-reference">[\s\S]*?<el-dropdown-item class="tool-config-dropdown-trigger"[\s\S]*?<cs name="caret-right"[\s\S]*?<!-- execution style -->/,
    'the quick-actions menu must place the auto-approval submenu after Skills and before execution style'
  )
  assert.doesNotMatch(
    inputArea,
    /<el-dropdown-item class="tool-config-dropdown-trigger" @click\.stop>/,
    'the auto-approval submenu trigger must let click events reach its popover reference'
  )
  assert.match(
    inputArea,
    /<template #reference>\s*<span class="execution-style-reference">[\s\S]*?<el-dropdown-item class="execution-style-dropdown-trigger"[\s\S]*?<cs name="caret-right"/,
    'the quick-actions execution-style submenu must use a concrete DOM popover reference'
  )
  assert.doesNotMatch(
    inputArea,
    /<el-dropdown-item class="execution-style-dropdown-trigger" @click\.stop>/,
    'the submenu trigger must let click events reach its popover reference'
  )
  assert.match(inputArea, /getExecutionStyleOptions\(props\.selectedAgent\)/)
  assert.match(inputArea, /emit\('update-personality', style\)/)
  assert.match(workflowView, /@update-personality="updateWorkflowPersonality"/)
  assert.match(
    workflowCore,
    /resolveExecutionStylePreference\(\s*personality,\s*getWorkflowAgentForExecutionStyle\(\)\s*\)/,
    'the workflow preference must be checked against the current Agent before persistence'
  )
})

test('workflow composer keeps Tab as four spaces outside suggestion selection', async () => {
  const input = await readFile('src/composables/workflow/useWorkflowInput.ts', 'utf8')

  assert.match(
    input,
    /if \(event\.key === 'Tab'\) \{[\s\S]*?event\.preventDefault\(\)[\s\S]*?const indentation = '    '[\s\S]*?inputMessage\.value = value\.slice\(0, start\) \+ indentation \+ value\.slice\(end\)/,
    'Tab must insert four spaces instead of moving focus in the composer'
  )
  assert.match(
    input,
    /if \(showSkillSuggestions\.value\) \{[\s\S]*?event\.key === 'Tab'[\s\S]*?onSkillSelect/,
    'Tab must continue selecting a visible skill suggestion'
  )
  assert.match(
    input,
    /if \(showFileSuggestions\.value\) \{[\s\S]*?event\.key === 'Tab'[\s\S]*?onFileSelect/,
    'Tab must continue selecting a visible file suggestion'
  )
})

test('applied compression clears the indicator and updates context usage', async () => {
  const workflowCore = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')

  assert.match(
    workflowCore,
    /payload\.type === 'compression_applied'[\s\S]*?setCompressionStatus\(sessionId, false, ''\)[\s\S]*?setCurrentContextTokens\([\s\S]*?payload\.current_context_tokens[\s\S]*?payload\.max_context_tokens/,
    'a persisted compression must clear the loading indicator and apply its authoritative usage'
  )
})

test('message resize observer is hoisted for immediate watchers', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /function syncMessageContentResizeObserver\(\)/,
    'the resize observer helper must use a hoisted function declaration'
  )
  assert.match(
    messageList,
    /watch\(\n  \[visibleMessages, collapsedMessages\][\s\S]*?syncMessageContentResizeObserver\(\)/,
    'the immediate message watcher must call the hoisted resize observer helper'
  )
})

test('off-bottom readers preserve a message window anchor while new messages render', async () => {
  const [workflowSessionPane, workflowMessages, messageList] = await Promise.all([
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  ])

  assert.match(
    workflowSessionPane,
    /@message-window-anchor-change="messageProjection\.setMessageWindowAnchor"/,
    'the workflow view must pass the reader anchor from the message list to the projection'
  )
  assert.match(
    workflowMessages,
    /selectVisibleWorkflowMessageWindow\([\s\S]*?loadedTaskMessageCount\.value,[\s\S]*?messageWindowAnchorId\.value/,
    'the upstream message window must retain the reported reader anchor'
  )
  assert.match(
    messageList,
    /:data-window-anchor-id="message\.windowAnchorId \|\| null"/,
    'rendered messages must expose the upstream-stable anchor identity'
  )
  assert.match(
    messageList,
    /const syncReadingScrollAnchor = \(\) => \{[\s\S]*?emit\('message-window-anchor-change', anchor\.windowAnchorId\)/,
    'scrolling away from the bottom must report the first readable persisted message'
  )
  assert.match(
    messageList,
    /const restoreReadingScrollAnchor = \(\) => \{[\s\S]*?container\.scrollTop \+= offsetDelta/,
    'message updates must restore the anchored message to its previous viewport offset'
  )
  assert.match(
    messageList,
    /messageContentResizeObserver = new ResizeObserver\(\(\) => \{[\s\S]*?restoreReadingScrollAnchor\(\)/,
    'asynchronous message height changes must also preserve the reader anchor'
  )
  assert.doesNotMatch(
    messageList,
    /slice\(-visibleMessageLimit\.value\)/,
    'the message list must not apply a second tail-only window that can discard the reader anchor'
  )
})

test('switching workflows clears an unobserved compression indicator from the previous session', async () => {
  const workflowCore = await readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8')

  assert.match(
    workflowCore,
    /if \(previousWorkflowId && previousWorkflowId !== id\) \{\s*setCompressionStatus\(previousWorkflowId, false, ''\)[\s\S]*?currentSessionId\.value = id/,
    'switching away must discard compression UI state whose completion event is no longer observed'
  )
})

test('context snapshots render the v2 handoff contract without losing legacy snapshot support', async () => {
  const [messageList, enLocale, zhHansLocale, zhHantLocale] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/i18n/locales/en.json', 'utf8'),
    readFile('src/i18n/locales/zh-Hans.json', 'utf8'),
    readFile('src/i18n/locales/zh-Hant.json', 'utf8')
  ])

  assert.match(messageList, /const getContextSnapshotV2Kind = snapshot =>/)
  assert.match(messageList, /\['pressure_handoff', 'completed_task_rollup'\]/)
  assert.match(messageList, /const formatV2ContextSnapshot = snapshot =>/)
  assert.match(messageList, /snapshot\.user_directives/)
  assert.match(messageList, /snapshot\.boundary_open_items/)
  assert.match(messageList, /snapshot\.unresolved_carryovers/)
  assert.match(messageList, /snapshot\.completed_work/)
  assert.match(messageList, /snapshot\.file_changes/)
  assert.match(messageList, /snapshot\.review_rounds/)
  assert.match(messageList, /const formatBoundaryOpenItems = items =>/)
  assert.match(messageList, /const formatReviewRound = review =>/)
  assert.match(messageList, /const findings = snapshotMarkdownList\(review\.findings\)/)
  assert.match(messageList, /const requiredFixes = snapshotMarkdownList\(review\.required_fixes\)/)
  assert.match(messageList, /const evidence = snapshotMarkdownList\(review\.evidence_refs\)/)
  assert.match(messageList, /\.filter\(section => section\?\.\[1\]\)/)
  assert.match(messageList, /const v2Display = formatV2ContextSnapshot\(parsed\)/)
  assert.match(messageList, /kind === 'pressure_handoff'/)
  assert.match(messageList, /kind === 'completed_task_rollup'/)
  assert.match(
    messageList,
    /\['Overall Goal', jsonSnapshotSectionText\(parsed\.overall_goal\)\]/,
    'legacy JSON snapshots must retain their formatter'
  )
  assert.match(messageList, /if \(!content\.includes\('<state_snapshot'\)\) return content/)

  for (const locale of [enLocale, zhHansLocale, zhHantLocale]) {
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"pressureHandoff"/)
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"completedTaskRollup"/)
    assert.match(locale, /"contextSnapshot": \{[\s\S]*?"reviewStatus"/)
  }
})

test('sandbox profile suggestions inherit the effective scheme runtime preference', async () => {
  const sandboxSettings = await readFile('src/components/setting/Sandbox.vue', 'utf8')

  assert.match(
    sandboxSettings,
    /const effectiveRuntimePreference = profile =>[\s\S]*?profile\?\.runtimePreference && profile\.runtimePreference !== 'auto'[\s\S]*?draft\.value\.config\?\.runtimePreference \|\| 'auto'/,
    'an auto Profile must inherit the scheme runtime preference used by the resolver'
  )
  assert.equal(
    sandboxSettings.match(/runtimeKeys\(effectiveRuntimePreference\(/g)?.length,
    3,
    'image candidates, instance candidates, and image-size tie-breaking must share the effective preference'
  )
  assert.doesNotMatch(
    sandboxSettings,
    /runtimeKeys\(profileDraft\.value\?\.runtimePreference \|\| 'auto'\)/,
    'a Docker scheme with an auto Profile must not present MSB candidates'
  )
})

test('cost analysis interaction is gated by accepted completion and terminal child summaries', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  const styles = await readFile('src/styles/workflow/messages.scss', 'utf8')

  assert.match(messageList, /!message\?\.isApproved \|\| message\?\.toolDisplay\?\.isError/)
  assert.match(messageList, /\['completed', 'failed', 'cancelled', 'interrupted'\]/)
  assert.match(messageList, /getSubAgentCostExpandId/)
  assert.match(messageList, /getFinishTaskCostExpandId/)
  assert.match(messageList, /finish-task-cost-card/)
  assert.doesNotMatch(messageList, /finish-task-display--in-card/)
  assert.match(styles, /margin-left: var\(--cs-space\)/)
  assert.match(styles, /:hover \.finish-task-cost-arrow/)
})

test('message history loading renders an Element Plus skeleton from the store loading state', async () => {
  const [messageList, workflowSessionPane, workflowStore, styles] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/components/workflow/WorkflowSessionMessagePane.vue', 'utf8'),
    readFile('src/stores/workflow.js', 'utf8'),
    readFile('src/styles/workflow/messages.scss', 'utf8')
  ])

  assert.match(messageList, /v-if="props\.isLoading" class="message-skeleton"/)
  assert.match(messageList, /<el-skeleton animated>/)
  assert.match(workflowSessionPane, /:is-loading="isLoadingMessages"/)
  assert.match(
    workflowStore,
    /const requestRevision = \+\+messageLoadRevision;\s*isLoadingMessages\.value = true;/
  )
  assert.match(
    workflowStore,
    /messages\.value = appendMissingPendingToolMessages\([\s\S]*?isLoadingMessages\.value = false;/
  )
  assert.match(styles, /\.message-skeleton/)
})

test('tool duration badges use structured backend metadata and preserve the requested exclusions', async () => {
  const [messageList, workflowEngine] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src-tauri/src/workflow/react/engine.rs', 'utf8')
  ])

  assert.match(messageList, /message\?\.metadata\?\.duration_ms/)
  assert.match(messageList, /toolName\.startsWith\('todo_'\)/)
  assert.match(messageList, /\['ask_user', 'submit_plan', 'submit_result', 'skill'\]/)
  assert.match(messageList, /durationMs <= 60_000/)
  assert.match(messageList, /\$\{minutes\}m\$\{seconds\}s/)
  assert.match(messageList, /getToolExecutionDurationLabel\(message\)[\s\S]*?class="shell-execution-route-badge"[\s\S]*?<cs v-if="message\.isApproved"/)
  assert.match(workflowEngine, /metadata\["duration_ms"\] = serde_json::json!\(duration_ms\)/)
  assert.match(workflowEngine, /fn should_expose_tool_duration\(tool_name: &str\)/)
})

test('persisted awaiting-user status restores answer controls when an older snapshot lacks wait_reason', async () => {
  const workflowStore = await readFile('src/stores/workflow.js', 'utf8')

  assert.match(
    workflowStore,
    /const persistedWaitReason =[\s\S]*?waitReason\.value =[\s\S]*?persistedWaitReason \|\|[\s\S]*?status === WORKFLOW_STATUSES\.AWAITING_USER[\s\S]*?WORKFLOW_WAIT_REASONS\.USER_INPUT/
  )
})

test('tool activity grouping keeps only explicit independent segments as boundaries', async () => {
  const [messageList, workflowMessages, projectionRules] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
    readFile('src/composables/workflow/messageProjectionRules.js', 'utf8')
  ])

  assert.match(
    projectionRules,
    /const currentIsThought = isThinkOnlyAssistantMessage\(current\)[\s\S]*?collapsed\.push\(buildToolGroupMessage\(\[\], index, thoughts, true\)\)/,
    'a thought without a prior visible tool group can become its own ongoing group'
  )
  assert.match(
    projectionRules,
    /currentIsThought && isStandaloneOngoingThoughtRun\(input, index\)/,
    'a thought followed by later tool activity must start grouped instead of flashing outside the tool group'
  )
  assert.match(
    projectionRules,
    /if \(getCollapsibleToolGroupKind\(message\) \|\| isCollapsedWorkflowToolGroupMessage\(message\)\) \{\s*return false/,
    'standalone ongoing thought groups must stop being standalone once tool activity appears after them'
  )
  assert.match(
    projectionRules,
    /while \(nextIndex < input\.length && !isToolGroupBoundaryMessage\(input\[nextIndex\]\)\)/,
    'non-independent thoughts and tools must be collected into one tool group until a boundary appears'
  )
  assert.match(
    projectionRules,
    /if \(isCompletionReportMessage\(message\) \|\| isWorkflowCompletionMessage\(message\)\) return true/,
    'completion tool messages must split tool groups so the finish-task badge remains visible after history is expanded'
  )
  assert.doesNotMatch(
    projectionRules,
    /isCollapsedWorkflowToolGroupMessage\(current\) && current\.metadata\?\.tool_group_is_ongoing/,
    'ongoing thought groups must remain eligible to merge back into adjacent non-independent tool activity'
  )
  assert.match(
    projectionRules,
    /!hasVisibleWorkflowText\(removeSystemReminder\(message\?\.message \|\| ''\)\) &&\s*hasVisibleWorkflowText\(message\?\.reasoning \|\| ''\)/,
    'only assistant messages without visible content may be treated as thought-only tool activity'
  )
  assert.doesNotMatch(
    messageList,
    /return isThinkStep \? !!\(content \|\| reasoning\) : !content && !!reasoning/,
    'stepType Think must not make visible assistant output collapse into a tool group'
  )
  assert.match(
    projectionRules,
    /if \(message\.role === 'assistant'\) \{\s*return \(\s*!isThinkOnlyAssistantMessage\(message\) &&\s*hasVisibleWorkflowText\(removeSystemReminder\(message\?\.message \|\| ''\)\)/,
    'a visible non-thought assistant text message must still split tool groups, while thought text does not'
  )
  assert.match(
    projectionRules,
    /const getToolGroupIcon = \(kind, groupMessages\) => \{\s*if \(groupMessages\.some\(isCollapsibleMutationToolMessage\)\) return TOOL_GROUP_ICONS\.mutation_tools/,
    'a mixed tool group containing file edits must prefer the edit icon'
  )
  assert.match(
    messageList,
    /const isStreamingThoughtMergedIntoToolGroup = message =>[\s\S]*?hasStreamingThoughtOnly\.value[\s\S]*?message === lastVisibleMessage\.value[\s\S]*?isCollapsedToolGroupMessage\(message\)[\s\S]*?\(message\.groupedTools\?\.length \|\| 0\) > 0/,
    'a reasoning-only stream must attach only to the final tool group, never an independent message or thought-only group'
  )
  assert.match(
    messageList,
    /const shouldShowStandaloneStreamingChat = computed\([\s\S]*?!isStreamingThoughtMergedIntoToolGroup\(lastVisibleMessage\.value\)/,
    'the standalone streaming card must stay hidden only while its thought is attached to the preceding tool group'
  )
  assert.match(
    messageList,
    /v-if="isStreamingThoughtMergedIntoToolGroup\(message\)"[\s\S]*?STREAMING_REASONING_ID/,
    'an attached streaming thought must retain its expandable reasoning UI inside the tool group'
  )
  assert.match(
    workflowMessages,
    /const toolGroupOrderById = new Map\(\)[\s\S]*?rawMsgs\.forEach\(\(message, messageIndex\) => \{[\s\S]*?toolGroupOrderById\.set\(toolCallId, messageIndex \+ \(callIndex \+ 1\) \/ \(callCount \+ 1\)\)/,
    'tool-call declaration order must be recorded before assistant tool messages can be hidden'
  )
  assert.match(
    workflowMessages,
    /id: call\.id,[\s\S]*?groupOrder:[\s\S]*?toolGroupOrderById\.get\(getToolCallId\(call\)\)/,
    'pending tool placeholders must carry their original tool-call order'
  )
  assert.match(
    workflowMessages,
    /message\.role === 'tool'[\s\S]*?toolGroupOrderById\.get\(String\(message\.metadata\?\.tool_call_id \|\| ''\)\.trim\(\)\) \?\? message\.sourceOrder/,
    'completed tool messages must carry the same original order even after their assistant declaration is filtered'
  )
  assert.match(
    projectionRules,
    /call\.groupOrder \?\? index \+ \(callIndex \+ 1\) \/ \(pendingCalls\.length \+ 1\)/,
    'the message projection must consume the stable pending-tool order and only fall back for old data'
  )
  assert.match(
    projectionRules,
    /tools\.push\(\{ \.\.\.message, groupOrder: message\.groupOrder \?\? index \}\)/,
    'tool collection must not overwrite a stable order with a transient projection index'
  )
  assert.match(
    workflowMessages,
    /const processedMsgs = rawMsgs\.map\(\(m, sourceOrder\) => \{[\s\S]*?sourceOrder \}/,
    'each live message must retain its durable transcript position through the display projection'
  )
  assert.match(
    workflowMessages,
    /message\.role === 'tool'[\s\S]*?\?\? message\.sourceOrder[\s\S]*?: message\.groupOrder \?\? message\.sourceOrder/,
    'thoughts and tools must share the same durable order axis while tool calls retain declaration order'
  )
  assert.match(
    projectionRules,
    /thoughts\.push\([\s\S]*?thought\.groupOrder \?\? index \+ thoughtIndex \/ 1000/,
    'nested tool groups must retain each thought’s durable order instead of replacing it with a transient group index'
  )
  assert.match(
    projectionRules,
    /thoughts\.push\(buildGroupedThoughtItem\(message, index, message\.groupOrder \?\? index\)\)/,
    'grouped thoughts must use their durable order instead of the transient filtered-list index'
  )
  assert.match(
    projectionRules,
    /const \{ thoughtOrders, toolOrders \} = getWorkflowToolGroupRenderOrders\(thoughts, groupMessages\)[\s\S]*?renderOrder: thoughtOrders\[thoughtIndex\][\s\S]*?renderOrder: toolOrders\[toolIndex\]/,
    'thoughts and tools must be assigned integer render ranks from their shared durable order axis'
  )
  assert.match(
    messageList,
    /:style="\{ order: thought\.renderOrder \?\? thoughtIndex \}"[\s\S]*?:style="\{ order: tool\.renderOrder \?\? toolIndex \}"/,
    'the expanded tool group must pass only integer render ranks to CSS flex order'
  )
})
