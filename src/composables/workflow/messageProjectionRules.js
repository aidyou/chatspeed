/**
 * Frontend workflow projection rules that must stay aligned with backend authority.
 *
 * Keep these rules centralized and covered by a lightweight Node test so future
 * UI refactors do not silently reintroduce transcript projection regressions.
 */

export const normalizeVisibleCompletionReport = value => {
  const visible = String(value ?? '').replace(
    /<think>[\s\S]*?<\/think>|<thought>[\s\S]*?<\/thought>|<(?:think|thought)>[\s\S]*$/gi,
    ''
  )

  return visible
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean)
    .filter(line => !['done', 'finished', 'complete', 'completed', 'task complete'].includes(line.toLowerCase()))
    .join('\n')
}

export const collectSubAgentCompletions = (visibleGroups = [], progressValues = []) => {
  const completions = new Map()

  for (const group of visibleGroups) {
    for (const message of group?.messages || []) {
      const meta = message?.metadata || {}
      const completionId = meta.sub_agent_id || meta.data?.sub_agent_id
      if (meta.observation_type !== 'sub_agent_completion' || !completionId) continue

      completions.set(completionId, {
        summary: meta.summary || '',
        execution_status: meta.execution_status || '',
        result: meta.result || {},
        sub_agent_name: meta.sub_agent_name || '',
        sub_agent_task: meta.sub_agent_task || '',
        data: meta.data || {}
      })
    }
  }

  for (const progress of progressValues) {
    const completionId = progress?.subAgentId || progress?.sub_agent_id || ''
    const result = progress?.result
    if (!completionId || !result || typeof result !== 'object') continue

    completions.set(completionId, {
      summary: progress.summary || result.summary || '',
      execution_status: progress.status || result.status || '',
      result,
      sub_agent_name: progress.agentName || progress.agent_name || '',
      sub_agent_task: progress.task || '',
      data: {}
    })
  }

  return completions
}

/**
 * Preserve explicit backend execution statuses for tool messages.
 *
 * Final review starts by persisting a `complete_workflow` tool
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
 * Read canonical tool identity only from structured workflow metadata.
 */
export const getStructuredWorkflowToolName = message => {
  const metadata = message?.metadata || message || {}
  return String(
    metadata.tool_name ||
      metadata.tool_call?.name ||
      metadata.tool_call?.function?.name ||
      ''
  )
    .trim()
    .toLowerCase()
}

export const isPendingApprovalEntryForTool = (entry, sessionId, toolName) => {
  const entryId = String(entry?.id || '').trim()
  const expectedToolName = String(toolName || '').trim().toLowerCase()
  return (
    !!expectedToolName &&
    entry?.sessionId === sessionId &&
    !!entryId &&
    entryId !== 'awaiting_approval' &&
    String(entry?.toolName || '').trim().toLowerCase() === expectedToolName
  )
}

/**
 * Project approval visibility exclusively from the canonical pending ID set.
 *
 * Transcript metadata describes the message itself, while the pending approval
 * collection describes the current workflow state. Do not infer current
 * approval visibility from titles, actions, command text, or stale message
 * statuses.
 */
export const isWorkflowMessagePendingApproval = (message, pendingApprovalIds = []) => {
  const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
  if (!toolCallId) return false

  const pendingIds =
    pendingApprovalIds instanceof Set
      ? pendingApprovalIds
      : new Set(
          (Array.isArray(pendingApprovalIds) ? pendingApprovalIds : [])
            .map(id => String(id || '').trim())
            .filter(Boolean)
        )

  return pendingIds.has(toolCallId)
}

/**
 * Distinguish an approved tool waiting for its turn from a tool that has
 * actually started. The local approved-submission flag covers the short interval
 * before the backend approval event updates the message metadata.
 */
export const isWorkflowToolAwaitingExecution = (message, approvedSubmission = false) => {
  const executionStatus = String(message?.metadata?.execution_status || '').toLowerCase()

  if (executionStatus === 'approval_submitted') return true
  if (['running', 'completed', 'failed', 'interrupted', 'rejected'].includes(executionStatus)) {
    return false
  }

  return Boolean(approvedSubmission)
}

/**
 * Identify the completion tool exclusively from structured metadata.
 *
 * Bash commands can legitimately contain strings such as `FinishTask` or
 * `InvalidFinishSummary`. Do not add title, action, localized-label, or message
 * content fallbacks here: they can hide another tool's approval UI behind the
 * completion-only presentation. Historical records without a structured tool
 * name intentionally use the generic tool presentation.
 */
export const isWorkflowCompletionMessage = message =>
  getStructuredWorkflowToolName(message) === 'complete_workflow'

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
