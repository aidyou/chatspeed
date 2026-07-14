/**
 * Frontend workflow projection rules that must stay aligned with backend authority.
 *
 * Keep these rules centralized and covered by a lightweight Node test so future
 * UI refactors do not silently reintroduce transcript projection regressions.
 */

/**
 * Preserve explicit backend execution statuses for tool messages.
 *
 * Final review starts by persisting a `complete_workflow_with_summary` tool
 * observation with `execution_status = "waiting"` and
 * `review_display_state = "final_review_pending"`. If frontend code rewrites
 * that non-terminal status to `completed`, the UI will rotate the task into the
 * completed bucket before the reviewer child actually resolves.
 */
export const inferWorkflowToolExecutionStatus = (message, existingMeta = {}) => {
  const explicitExecutionStatus = existingMeta?.execution_status ?? message?.metadata?.execution_status
  const isError = message?.isError || message?.is_error || message?.metadata?.is_error
  const approvalStatus = message?.metadata?.approval_status

  if (typeof explicitExecutionStatus === 'string' && explicitExecutionStatus.trim()) {
    return explicitExecutionStatus
  }
  if (approvalStatus === 'rejected') return 'rejected'
  if (isError) return 'failed'
  if (approvalStatus === 'pending') return 'pending_approval'

  // Incoming tool messages without an explicit execution status are durable
  // terminal observations from the backend.
  return 'completed'
}

/**
 * Decide whether a workflow message should render as a delegated-task card.
 *
 * Final review pending messages are persisted on the completion tool
 * observation, not on a `sub_agent_run` tool row. We therefore must not key the
 * card purely on `tool_name === "sub_agent_run"`; any message carrying the
 * child-session identity for the reviewer should keep the card visible.
 */
export const shouldRenderSubAgentCard = message => {
  if (!message?.subAgentCard) return false

  const metadata = message?.metadata || {}
  const toolName = String(metadata.tool_name || '').toLowerCase()
  const reviewDisplayState = String(metadata.review_display_state || '').toLowerCase()
  const subAgentId = metadata.sub_agent_id || metadata.subAgentId || null

  return (
    toolName === 'sub_agent_run' ||
    reviewDisplayState === 'final_review_pending' ||
    !!subAgentId
  )
}
