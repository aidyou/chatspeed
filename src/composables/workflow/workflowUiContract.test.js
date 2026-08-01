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

test('cost analysis interaction is gated by accepted completion and terminal child summaries', async () => {
  const messageList = await readFile('src/components/workflow/WorkflowMessageList.vue', 'utf8')
  const styles = await readFile('src/styles/workflow/messages.scss', 'utf8')

  assert.match(messageList, /!message\?\.isApproved \|\| message\?\.toolDisplay\?\.isError/)
  assert.match(messageList, /\['completed', 'failed', 'cancelled', 'interrupted'\]/)
  assert.match(messageList, /getSubAgentCostExpandId/)
  assert.match(messageList, /getFinishTaskCostExpandId/)
  assert.match(messageList, /finish-task-display--in-card/)
  assert.match(styles, /margin-left: 15px/)
  assert.match(styles, /:hover \.finish-task-cost-arrow/)
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
  assert.doesNotMatch(
    messageList,
    /isCollapsedToolGroupMessage\(current\) && current\.metadata\?\.tool_group_is_ongoing/,
    'ongoing thought groups must remain eligible to merge back into adjacent non-independent tool activity'
  )
  assert.match(
    messageList,
    /if \(message\.role === 'assistant'\) \{\s*return !!props\.removeSystemReminder\(message\?\.message \|\| ''\)\.trim\(\)/,
    'a visible assistant text message must split tool groups, so an immediately preceding thought is not swallowed'
  )
  assert.match(
    messageList,
    /const getToolGroupIcon = \(kind, messages\) => \{\s*if \(messages\.some\(isCollapsibleMutationToolMessage\)\) return TOOL_GROUP_ICONS\.mutation_tools/,
    'a mixed tool group containing file edits must prefer the edit icon'
  )
})
