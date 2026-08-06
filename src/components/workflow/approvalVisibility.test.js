import assert from 'node:assert/strict'

import {
  isStructuredPendingApproval,
  isToolPendingApprovalVisible,
  shouldRenderInlineApprovalWithoutExpansion,
  shouldShowInlineApprovalForMessage
} from './approvalVisibility.js'

const pendingMessage = {
  metadata: {
    tool_call_id: 'tool_571ae521',
    approval_status: 'pending',
    execution_status: 'pending_approval'
  }
}

assert.equal(
  isStructuredPendingApproval(pendingMessage),
  true,
  'structured pending metadata must remain sufficient to render approval UI'
)

assert.equal(
  isToolPendingApprovalVisible(pendingMessage, []),
  true,
  'pending tool visibility must not depend solely on external pending id reconciliation'
)

assert.equal(
  shouldRenderInlineApprovalWithoutExpansion(pendingMessage),
  true,
  'structured pending tools must expose inline approval UI without requiring expansion state'
)

assert.equal(
  isToolPendingApprovalVisible(
    {
      metadata: {
        tool_call_id: 'tool_571ae521',
        approval_status: 'approved',
        execution_status: 'approval_submitted'
      }
    },
    []
  ),
  false,
  'approval-submitted tools must not be treated as pending approvals'
)

assert.equal(
  isToolPendingApprovalVisible(
    {
      metadata: {
        tool_call_id: 'tool_571ae521',
        approval_status: 'approved',
        execution_status: 'completed'
      }
    },
    ['tool_571ae521']
  ),
  true,
  'legacy pending approval ids remain a compatibility path'
)

assert.equal(
  shouldShowInlineApprovalForMessage({
    message: pendingMessage,
    isPending: true,
    isSubmitting: false
  }),
  true,
  'ordinary pending approvals remain visible'
)

console.log('approvalVisibility tests passed')
