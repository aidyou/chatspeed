export const isStructuredPendingApproval = message => {
  const meta = message?.metadata || {}
  const approvalStatus = String(meta.approval_status || '').toLowerCase()
  const executionStatus = String(meta.execution_status || '').toLowerCase()

  if (approvalStatus !== 'pending' && executionStatus !== 'pending_approval') {
    return false
  }

  return ![
    'approval_submitted',
    'running',
    'rejected',
    'completed',
    'failed',
    'interrupted'
  ].includes(executionStatus)
}

export const isToolPendingApprovalVisible = (message, pendingApprovalIds = []) => {
  const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
  if (!toolCallId) return false

  const pendingIds = Array.isArray(pendingApprovalIds)
    ? pendingApprovalIds.map(id => String(id || '').trim()).filter(Boolean)
    : []

  return pendingIds.includes(toolCallId) || isStructuredPendingApproval(message)
}

export const shouldShowHostFallbackConfirmation = (pendingApproval, isSubmitting = false) =>
  pendingApproval?.details?.approval_kind === 'host_fallback' && !isSubmitting

export const shouldShowInlineApprovalForMessage = ({
  message,
  isPending = false,
  isSubmitting = false,
  activeHostFallbackToolCallId = ''
} = {}) => {
  const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
  return (
    isPending &&
    message?.metadata?.details?.approval_kind !== 'host_fallback' &&
    !isSubmitting &&
    String(activeHostFallbackToolCallId || '').trim() !== toolCallId
  )
}

export const shouldRenderInlineApprovalWithoutExpansion = message =>
  isStructuredPendingApproval(message)
