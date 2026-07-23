import assert from 'node:assert/strict'

import {
  collectSubAgentCompletions,
  getStructuredWorkflowToolName,
  inferWorkflowToolExecutionStatus,
  isPendingApprovalEntryForTool,
  isWorkflowCompletionMessage,
  isWorkflowToolAwaitingExecution,
  normalizeVisibleCompletionReport,
  shouldRenderSubAgentCard
} from './messageProjectionRules.js'

const finalReviewPendingMessage = {
  metadata: {
    execution_status: 'waiting',
    review_display_state: 'final_review_pending',
    sub_agent_id: 'subagent_final_review_1'
  },
  subAgentCard: {
    status: 'running'
  }
}

assert.equal(
  inferWorkflowToolExecutionStatus(finalReviewPendingMessage, finalReviewPendingMessage.metadata),
  'waiting',
  'explicit backend waiting status must survive frontend projection'
)

assert.equal(
  shouldRenderSubAgentCard(finalReviewPendingMessage),
  true,
  'final review pending messages with a child-session id must render the delegated-task card'
)

assert.equal(
  shouldRenderSubAgentCard({
    metadata: {
      tool_name: 'complete_workflow'
    },
    subAgentCard: null
  }),
  false,
  'messages without an assembled sub-agent card must not render as delegated-task cards'
)

assert.equal(
  inferWorkflowToolExecutionStatus(
    {
      metadata: {
        approval_status: 'pending'
      }
    },
    {
      approval_status: 'pending'
    }
  ),
  'pending_approval',
  'pending approvals without an explicit execution status should still map to pending_approval'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'approved',
        execution_status: 'approval_submitted'
      }
    },
    false
  ),
  true,
  'approval-submitted tools must render as awaiting execution before tool_started'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'pending',
        execution_status: 'pending_approval'
      }
    },
    true
  ),
  true,
  'the local submission flag must cover the interval before approval metadata reconciliation'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'approved',
        execution_status: 'running'
      }
    },
    true
  ),
  false,
  'the backend running state must take precedence over a stale local submission flag'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'rejected',
        execution_status: 'rejected'
      }
    },
    true
  ),
  false,
  'terminal backend states must take precedence over a stale local submission flag'
)

assert.equal(
  getStructuredWorkflowToolName({
    metadata: {
      title: 'Read write edit list bash grep glob web search Ask User FinishTask'
    }
  }),
  '',
  'display titles must never be interpreted as structured tool identity'
)

assert.equal(
  getStructuredWorkflowToolName({
    metadata: {
      tool_call: {
        function: {
          name: 'BASH'
        }
      },
      title: 'Submit Plan'
    }
  }),
  'bash',
  'structured tool identity must take precedence over unrelated display text'
)

assert.equal(
  isPendingApprovalEntryForTool(
    {
      id: 'tool_bash',
      sessionId: 'session-1',
      toolName: 'bash',
      action: 'Run a command containing submit plan'
    },
    'session-1',
    'submit_plan'
  ),
  false,
  'approval actions containing plan text must not be selected as submit_plan'
)

assert.equal(
  isPendingApprovalEntryForTool(
    {
      id: 'tool_plan',
      sessionId: 'session-1',
      toolName: 'submit_plan',
      action: 'Localized plan approval title'
    },
    'session-1',
    'submit_plan'
  ),
  true,
  'plan approval selection must use exact structured identity and session scope'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {
        tool_name: 'bash',
        execution_status: 'pending_approval',
        approval_status: 'pending'
      },
      toolDisplay: {
        action:
          'Run sqlite3 chatspeed.db "SELECT InvalidFinishSummary, FinishTask FROM workflow_messages"'
      }
    }
  ),
  false,
  'bash commands containing Finish markers must keep their approval presentation'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {
        tool_name: 'complete_workflow'
      },
      toolDisplay: {
        action: 'Finish task'
      }
    }
  ),
  true,
  'structured complete_workflow messages must use the completion presentation'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {},
      toolDisplay: {
        action: 'Finish task'
      }
    }
  ),
  false,
  'messages without structured tool identity must never use completion presentation'
)

const visibleCompletion = collectSubAgentCompletions(
  [
    {
      messages: [
        {
          metadata: {
            observation_type: 'sub_agent_completion',
            sub_agent_id: 'visible_background',
            execution_status: 'completed',
            result: { result: 'visible result' }
          }
        }
      ]
    }
  ],
  [
    {
      subAgentId: 'live_background',
      status: 'completed',
      result: { status: 'completed', result: 'live result' }
    }
  ]
)
assert.equal(visibleCompletion.get('visible_background').result.result, 'visible result')
assert.equal(visibleCompletion.get('live_background').result.result, 'live result')
assert.equal(
  visibleCompletion.has('hidden_history'),
  false,
  'completion projection must not scan messages outside visible task groups'
)

assert.equal(
  normalizeVisibleCompletionReport(
    '<THINK>Internal reasoning must not be rendered.</THINK>\nCompleted the requested change.\n<ThOuGhT>More internal reasoning.</ThOuGhT>\nVerified the targeted tests pass.'
  ),
  'Completed the requested change.\nVerified the targeted tests pass.',
  'completion report projection must remove mixed-case reasoning blocks before rendering'
)
assert.equal(
  normalizeVisibleCompletionReport('<thought>Reasoning only must not be rendered.</thought>'),
  '',
  'reasoning-only completion summaries must not render'
)

console.log('messageProjectionRules tests passed')
