import assert from 'node:assert/strict'

import {
  inferWorkflowToolExecutionStatus,
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
      tool_name: 'complete_workflow_with_summary'
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

console.log('messageProjectionRules tests passed')
