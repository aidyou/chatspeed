import assert from 'node:assert/strict'

import {
  appendMissingPendingToolMessages,
  deriveInlinePendingApprovals,
  detectApprovalRecoveryDrift,
  resolveExecutionContextPendingTool
} from './workflowApprovalRecovery.js'

const approvalWaitingStatuses = ['awaiting_approval', 'awaiting_auto_approval']

const executionContext = {
  wait_reason: 'approval',
  pending_tools: [
    {
      tool_call_id: 'tool_571ae521',
      tool_name: 'bash',
      arguments: { command: 'sqlite3 workflow.db' },
      details: {
        command: 'sqlite3 workflow.db',
        description: 'Inspect workflow state'
      },
      display_type: 'text'
    }
  ]
}

assert.deepEqual(
  resolveExecutionContextPendingTool(executionContext, 'tool_571ae521')?.pendingTools,
  [],
  'a structured resolution event must remove its tool from the live execution-context cache'
)

const historicalMessages = [
  {
    id: 63656,
    sessionId: 'session-1',
    role: 'assistant',
    metadata: {
      tool_calls: [
        {
          id: 'complete_1',
          function: {
            name: 'complete_workflow',
            arguments: { summary: 'Done' }
          }
        }
      ]
    }
  },
  {
    id: 63658,
    sessionId: 'session-1',
    role: 'tool',
    metadata: {
      tool_call_id: 'complete_1',
      tool_name: 'complete_workflow',
      execution_status: 'completed'
    }
  }
]

const inlineApprovals = deriveInlinePendingApprovals({
  currentWorkflowId: 'session-1',
  workflowTitle: 'Approval recovery',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext,
  messages: historicalMessages,
  approvalWaitingStatuses
})

assert.equal(inlineApprovals.length, 1, 'pending tools must recover from execution context')
assert.equal(inlineApprovals[0].toolCallId, 'tool_571ae521')
assert.equal(inlineApprovals[0].toolName, 'bash')
assert.deepEqual(inlineApprovals[0].details, {
  command: 'sqlite3 workflow.db',
  description: 'Inspect workflow state'
})

const legacyTitleOnlyApproval = deriveInlinePendingApprovals({
  currentWorkflowId: 'session-legacy',
  workflowTitle: 'Legacy approval',
  status: 'awaiting_approval',
  waitReason: 'approval',
  executionContext: null,
  messages: [
    {
      sessionId: 'session-legacy',
      role: 'tool',
      metadata: {
        tool_call_id: 'tool_legacy',
        title: 'Submit Plan after running bash search',
        approval_status: 'pending',
        execution_status: 'pending_approval'
      }
    }
  ],
  approvalWaitingStatuses
})

assert.equal(legacyTitleOnlyApproval.length, 1)
assert.equal(
  legacyTitleOnlyApproval[0].toolName,
  'unknown',
  'legacy display titles must not be promoted into canonical tool identity'
)
assert.equal(
  legacyTitleOnlyApproval[0].action,
  'Submit Plan after running bash search',
  'legacy titles may remain presentation-only labels'
)

const hydratedMessages = appendMissingPendingToolMessages({
  messages: historicalMessages,
  sessionId: 'session-1',
  executionContext,
  getPendingSummary: () => 'Awaiting approval'
})

assert.equal(
  hydratedMessages.filter(message => message?.metadata?.tool_call_id === 'tool_571ae521').length,
  1,
  'frontend hydration must synthesize one canonical pending tool message when transcript lacks it'
)
assert.deepEqual(
  hydratedMessages.find(message => message?.metadata?.tool_call_id === 'tool_571ae521')?.metadata,
  {
    tool_call_id: 'tool_571ae521',
    tool_name: 'bash',
    tool_call: {
      id: 'tool_571ae521',
      function: {
        name: 'bash',
        arguments: { command: 'sqlite3 workflow.db' }
      }
    },
    details: {
      command: 'sqlite3 workflow.db',
      description: 'Inspect workflow state'
    },
    display_type: 'text',
    summary: 'Awaiting approval',
    approval_status: 'pending',
    execution_status: 'pending_approval'
  },
  'synthetic pending messages must carry canonical approval metadata'
)

assert.deepEqual(
  deriveInlinePendingApprovals({
    currentWorkflowId: 'session-1',
    workflowTitle: 'Approval recovery',
    status: 'thinking',
    waitReason: null,
    executionContext,
    messages: hydratedMessages,
    approvalWaitingStatuses
  }),
  [],
  'inline approvals must be empty outside approval wait even if old pending messages remain in transcript'
)

assert.deepEqual(
  deriveInlinePendingApprovals({
    currentWorkflowId: 'session-1',
    workflowTitle: 'Approval recovery',
    status: 'awaiting_approval',
    waitReason: 'approval',
    executionContext,
    messages: [
      ...hydratedMessages,
      {
        id: 63713,
        sessionId: 'session-1',
        role: 'tool',
        metadata: {
          tool_call_id: 'tool_571ae521',
          tool_name: 'bash',
          approval_status: 'approved',
          execution_status: 'running'
        }
      }
    ],
    approvalWaitingStatuses
  }),
  [],
  'latest structured state for the same tool_call_id must resolve the approval item'
)

assert.deepEqual(
  detectApprovalRecoveryDrift({
    status: 'awaiting_approval',
    waitReason: 'approval',
    executionContext,
    inlinePendingApprovals: [],
    approvalWaitingStatuses
  }),
  {
    status: 'awaiting_approval',
    waitReason: 'approval',
    pendingToolIds: ['tool_571ae521']
  },
  'drift detection must fire when approval wait has pending tools but no inline approvals'
)

console.log('workflowApprovalRecovery tests passed')
