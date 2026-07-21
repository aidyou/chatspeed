import assert from 'node:assert/strict'

import {
  collectSubAgentCompletions,
  inferWorkflowToolExecutionStatus,
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
