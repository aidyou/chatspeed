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

console.log('workflow UI contract tests passed')

test('ask-user responses stay in the workflow chain but are hidden from the transcript UI', async () => {
  const [workflowView, workflowCore, workflowMessages, messageList, workflowEngine] =
    await Promise.all([
      readFile('src/views/Workflow.vue', 'utf8'),
      readFile('src/composables/workflow/useWorkflowCore.ts', 'utf8'),
      readFile('src/composables/workflow/useWorkflowMessages.ts', 'utf8'),
      readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
      readFile('src-tauri/src/workflow/react/engine.rs', 'utf8')
    ])

  assert.match(
    workflowView,
    /const submitAskUserResponse = async content => \{[\s\S]*?coreOnSendMessage\(content, \{\s*metadata: \{ ui_visibility: 'hide' \}/,
    'ask-user answers must use the existing hidden-message metadata instead of content-based filtering'
  )
  assert.match(
    workflowCore,
    /if \(options\.metadata\) \{\s*signalPayload\.metadata = options\.metadata/,
    'hidden-message metadata must continue through the user-message signal sent to the runtime'
  )
  const awaitingUserMetadataWrites = workflowEngine.match(
    /WorkflowSignal::UserMessage \{\s*content, metadata, \.\.\s*\}[\s\S]*?add_message_and_notify_internal\([\s\S]*?metadata,\s*\)/g
  )
  assert.equal(
    awaitingUserMetadataWrites?.length,
    2,
    'both awaiting-user signal branches must persist the structured UI metadata with the answer'
  )
  assert.match(
    workflowMessages,
    /m\.metadata\?\.ui_visibility === 'hide'[\s\S]*?return false/,
    'the workflow message projection must hide messages marked with ui_visibility=hide'
  )
  assert.match(
    messageList,
    /const uiVisibility = message\?\.metadata\?\.ui_visibility\s*if \(uiVisibility === 'hide'\) return true/,
    'the message list must retain its existing hidden-message safeguard'
  )
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
  assert.match(styles, /margin-left: 15px/)
  assert.match(styles, /:hover \.finish-task-cost-arrow/)
})

test('message history loading renders an Element Plus skeleton from the store loading state', async () => {
  const [messageList, workflowView, workflowStore, styles] = await Promise.all([
    readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8'),
    readFile('src/views/Workflow.vue', 'utf8'),
    readFile('src/stores/workflow.js', 'utf8'),
    readFile('src/styles/workflow/messages.scss', 'utf8')
  ])

  assert.match(messageList, /v-if="props\.isLoading" class="message-skeleton"/)
  assert.match(messageList, /<el-skeleton animated>/)
  assert.match(workflowView, /:is-loading="workflowStore\.isLoadingMessages"/)
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
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')

  assert.match(
    messageList,
    /const currentIsPendingThought = isThinkOnlyAssistantMessage\(current\)[\s\S]*?collapsed\.push\(buildToolGroupMessage\(\[\], index, thoughts, true\)\)/,
    'a thought without a prior visible tool group can become its own ongoing group'
  )
  assert.match(
    messageList,
    /currentIsPendingThought && isStandaloneOngoingThoughtRun\(messages, index\)/,
    'a thought followed by later tool activity must start grouped instead of flashing outside the tool group'
  )
  assert.match(
    messageList,
    /if \(getCollapsibleToolGroupKind\(message\) \|\| isCollapsedToolGroupMessage\(message\)\) return false/,
    'standalone ongoing thought groups must stop being standalone once tool activity appears after them'
  )
  assert.match(
    messageList,
    /while \(nextIndex < messages\.length && !isToolGroupBoundaryMessage\(messages\[nextIndex\]\)\)/,
    'non-independent thoughts and tools must be collected into one tool group until a boundary appears'
  )
  assert.match(
    messageList,
    /if \(isCompletionReportMessage\(message\) \|\| isFinishTaskMessage\(message\)\) return true/,
    'completion tool messages must split tool groups so the finish-task badge remains visible after history is expanded'
  )
  assert.doesNotMatch(
    messageList,
    /isCollapsedToolGroupMessage\(current\) && current\.metadata\?\.tool_group_is_ongoing/,
    'ongoing thought groups must remain eligible to merge back into adjacent non-independent tool activity'
  )
  assert.match(
    messageList,
    /return !content && !!reasoning/,
    'only assistant messages without visible content may be treated as thought-only tool activity'
  )
  assert.doesNotMatch(
    messageList,
    /return isThinkStep \? !!\(content \|\| reasoning\) : !content && !!reasoning/,
    'stepType Think must not make visible assistant output collapse into a tool group'
  )
  assert.match(
    messageList,
    /if \(message\.role === 'assistant'\) \{\s*return \([\s\S]*?!isThinkOnlyAssistantMessage\(message\) && !!props\.removeSystemReminder\(message\?\.message \|\| ''\)\.trim\(\)[\s\S]*?\)/,
    'a visible non-thought assistant text message must still split tool groups, while thought text does not'
  )
  assert.match(
    messageList,
    /const getToolGroupIcon = \(kind, messages\) => \{\s*if \(messages\.some\(isCollapsibleMutationToolMessage\)\) return TOOL_GROUP_ICONS\.mutation_tools/,
    'a mixed tool group containing file edits must prefer the edit icon'
  )
})
