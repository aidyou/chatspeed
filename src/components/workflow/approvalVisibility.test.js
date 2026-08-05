import assert from 'node:assert/strict'

import {
  isStructuredPendingApproval,
  isToolPendingApprovalVisible,
  shouldRenderInlineApprovalWithoutExpansion,
  shouldShowHostFallbackConfirmation,
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

const hostFallbackPending = {
  toolCallId: 'tool_host_fallback',
  details: { approval_kind: 'host_fallback' }
}
const staleShellApprovalMessage = {
  metadata: {
    tool_call_id: 'tool_host_fallback',
    details: { approval_kind: 'shell_command' }
  }
}

assert.equal(
  shouldShowHostFallbackConfirmation(hostFallbackPending, false),
  true,
  'structured Host fallback pending state must show the one-time confirmation'
)
for (const action of ['approve', 'reject']) {
  assert.equal(
    shouldShowHostFallbackConfirmation(hostFallbackPending, true),
    false,
    `Host fallback confirmation must hide immediately while ${action} submission is in flight`
  )
  assert.equal(
    shouldShowInlineApprovalForMessage({
      message: staleShellApprovalMessage,
      isPending: true,
      isSubmitting: true,
      activeHostFallbackToolCallId: ''
    }),
    false,
    `the stale shell approval must stay hidden during Host fallback ${action} submission`
  )
  assert.equal(
    shouldShowHostFallbackConfirmation(hostFallbackPending, false),
    true,
    `after ${action} signal failure clears submission, the restored pending fallback must be actionable again`
  )
  assert.equal(
    shouldShowInlineApprovalForMessage({
      message: staleShellApprovalMessage,
      isPending: true,
      isSubmitting: false,
      activeHostFallbackToolCallId: 'tool_host_fallback'
    }),
    false,
    `when ${action} failure restores the fallback confirmation, the stale first-stage shell approval must remain hidden`
  )
}

console.log('approvalVisibility tests passed')
