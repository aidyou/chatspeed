import assert from 'node:assert/strict'

import {
  isWorkflowCompletionMessage,
  isWorkflowMessagePendingApproval
} from './messageProjectionRules.js'
import { deriveInlinePendingApprovals } from '../../stores/workflowApprovalRecovery.js'

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
