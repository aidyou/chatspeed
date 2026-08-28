import { isWorkflowMcpTool } from './toolClassification.js'

// Models occasionally emit zero-width formatting characters as an otherwise empty message.
// They are not removed by String.prototype.trim(), so exclude them from visibility checks.
const INVISIBLE_WORKFLOW_CHARACTERS = /[\u200B-\u200D\u2060\uFEFF]/g

export const normalizeWorkflowTextForVisibility = value =>
  String(value ?? '').replace(INVISIBLE_WORKFLOW_CHARACTERS, '')

export const hasVisibleWorkflowText = value =>
  normalizeWorkflowTextForVisibility(value).trim().length > 0

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

export const resolveWorkflowPhaseFromPlanningMode = (planningMode, configuredPhase) => {
  if (planningMode) return 'planning'
  return String(configuredPhase || '').toLowerCase() === 'implementation'
    ? 'implementation'
    : 'standard'
}

export const isWorkflowManualClearContextMessage = message =>
  message?.role === 'system' &&
  message?.messageKind === 'summary' &&
  message?.messageSubtype === 'manual_clear_context'

const getWorkflowMessageIdentityDiscriminator = message => {
  const metadata = message?.metadata || {}
  const toolCallId = String(metadata.tool_call_id || metadata.toolCallId || '').trim()
  return [
    message?.role || '',
    message?.messageKind || message?.message_kind || metadata.message_kind || metadata.messageKind || '',
    message?.messageSubtype || message?.message_subtype || metadata.subtype || '',
    toolCallId
  ].join(':')
}

const hasSameWorkflowMessageIdentity = (left, right) => {
  const leftId = left?.id ?? left?.displayId
  const rightId = right?.id ?? right?.displayId
  if (leftId !== null && leftId !== undefined && rightId !== null && rightId !== undefined) {
    return (
      String(leftId) === String(rightId) &&
      getWorkflowMessageIdentityDiscriminator(left) === getWorkflowMessageIdentityDiscriminator(right)
    )
  }
  return left === right
}

/**
 * Keep manual clear-context dividers on the hidden side of the display boundary.
 * The marker starts a new context segment, but visually belongs to the task group
 * before it so opening a new task does not leave an orphan divider above the
 * active task.
 */
export const mergeManualClearContextMarkersIntoPreviousGroups = (
  groups = [],
  buildGroupId = groupMessages => groupMessages[0]?.id || ''
) => {
  const mergedGroups = []

  for (const group of groups) {
    const messages = group?.messages || []
    let markerCount = 0
    while (
      markerCount < messages.length &&
      isWorkflowManualClearContextMessage(messages[markerCount])
    ) {
      markerCount += 1
    }

    if (!markerCount || !mergedGroups.length) {
      mergedGroups.push(group)
      continue
    }

    const markers = messages.slice(0, markerCount)
    const previousGroup = mergedGroups[mergedGroups.length - 1]
    const previousMessages = previousGroup.messages || []
    const newMarkers = markers.filter(
      marker =>
        !previousMessages.some(previousMessage =>
          hasSameWorkflowMessageIdentity(previousMessage, marker)
        )
    )
    const nextPreviousMessages = [...previousMessages, ...newMarkers]
    mergedGroups[mergedGroups.length - 1] = {
      ...previousGroup,
      id: buildGroupId(nextPreviousMessages),
      messages: nextPreviousMessages
    }

    const remainingMessages = messages.slice(markerCount)
    if (remainingMessages.length) {
      mergedGroups.push({
        ...group,
        id: buildGroupId(remainingMessages),
        messages: remainingMessages
      })
    }
  }

  return mergedGroups
}

export const excludeManualClearContextMarkers = (messages = []) =>
  messages.filter(message => !isWorkflowManualClearContextMessage(message))

export const excludeLeadingManualClearContextMarkers = (messages = []) => {
  const firstContentIndex = messages.findIndex(
    message => !isWorkflowManualClearContextMessage(message)
  )
  if (firstContentIndex < 0) return []

  return messages.filter(
    (message, index) =>
      !isWorkflowManualClearContextMessage(message) || index > firstContentIndex
  )
}

export const isWorkflowTaskBoundaryMessage = message =>
  isWorkflowCompletionMessage(message) || isWorkflowManualClearContextMessage(message)

export const excludeLeadingWorkflowTaskBoundaryMessages = (messages = []) => {
  const firstContentIndex = messages.findIndex(message => !isWorkflowTaskBoundaryMessage(message))
  return firstContentIndex < 0 ? [] : messages.slice(firstContentIndex)
}

export const buildWorkflowTaskGroups = (
  messages = [],
  {
    buildGroupId = groupMessages => groupMessages[0]?.id || '',
    isCompletionBoundary = isWorkflowCompletionMessage,
    preserveLeadingBoundaries = false
  } = {}
) => {
  const groups = []
  let currentMessages = []

  const pushGroup = isCompleted => {
    if (!currentMessages.length) return
    const isBoundaryOnly = currentMessages.every(isWorkflowTaskBoundaryMessage)
    if (isBoundaryOnly) {
      if (groups.length) {
        const previousGroup = groups[groups.length - 1]
        const previousMessages = [...previousGroup.messages, ...currentMessages]
        groups[groups.length - 1] = {
          ...previousGroup,
          id: buildGroupId(previousMessages),
          messages: previousMessages
        }
      } else if (preserveLeadingBoundaries) {
        groups.push({
          id: buildGroupId(currentMessages),
          isCompleted,
          messages: currentMessages
        })
      }
    } else {
      groups.push({
        id: buildGroupId(currentMessages),
        isCompleted,
        messages: currentMessages
      })
    }
    currentMessages = []
  }

  for (const message of messages) {
    currentMessages.push(message)
    if (isCompletionBoundary(message) || isWorkflowManualClearContextMessage(message)) {
      pushGroup(true)
    }
  }
  pushGroup(false)

  return groups
}

export const hasOpenWorkflowTaskFrame = (completedGroups = [], activeMessages = []) => {
  if (activeMessages.length) return true
  const latestCompletedMessages = completedGroups[completedGroups.length - 1]?.messages || []
  return isWorkflowManualClearContextMessage(
    latestCompletedMessages[latestCompletedMessages.length - 1]
  )
}

export const selectVisibleWorkflowTaskGroups = (
  completedGroups = [],
  activeGroup = null,
  visibleGroupCount = 1,
  hasOpenTaskFrame = Boolean(activeGroup)
) => {
  const hasVisibleActiveGroup = hasOpenTaskFrame && Boolean(activeGroup)
  const completedLimit = Math.max(0, visibleGroupCount - (hasVisibleActiveGroup ? 1 : 0))
  const visibleCompletedGroups = completedLimit
    ? completedGroups.slice(-completedLimit)
    : []

  return activeGroup ? [...visibleCompletedGroups, activeGroup] : visibleCompletedGroups
}

/**
 * Limit the message projection before expensive display enhancement and Vue rendering.
 * Whole groups keep their original references so completed-group caches remain reusable.
 */
export const getWorkflowMessageWindowAnchorId = message => {
  const persistedId = message?.id
  if (persistedId !== null && persistedId !== undefined && persistedId !== '') {
    return `${String(persistedId)}:${getWorkflowMessageIdentityDiscriminator(message)}`
  }

  const metadata = message?.metadata || {}
  const stableId = String(
    metadata.tool_call_id ||
      metadata.toolCallId ||
      metadata.queued_user_message_id ||
      metadata.queuedUserMessageId ||
      metadata.client_message_id ||
      metadata.clientMessageId ||
      ''
  ).trim()

  return stableId ? `${stableId}:${getWorkflowMessageIdentityDiscriminator(message)}` : ''
}

export const selectVisibleWorkflowMessageWindow = (
  groups = [],
  visibleMessageCount = 200,
  windowAnchorId = ''
) => {
  const normalizedLimit = Math.max(0, Math.floor(Number(visibleMessageCount) || 0))
  const totalMessageCount = groups.reduce(
    (total, group) => total + (group?.messages?.length || 0),
    0
  )
  const normalizedAnchorId = String(windowAnchorId || '').trim()
  let anchorIndex = -1
  let messageIndex = 0

  if (normalizedAnchorId) {
    for (const group of groups) {
      for (const message of group?.messages || []) {
        if (getWorkflowMessageWindowAnchorId(message) === normalizedAnchorId) {
          anchorIndex = messageIndex
          break
        }
        messageIndex += 1
      }
      if (anchorIndex >= 0) break
    }
  }

  let remainingHiddenCount = Math.max(0, totalMessageCount - normalizedLimit)
  if (anchorIndex >= 0) {
    remainingHiddenCount = Math.min(remainingHiddenCount, anchorIndex)
  }
  const visibleGroups = []

  for (const group of groups) {
    const messages = group?.messages || []
    if (remainingHiddenCount >= messages.length) {
      remainingHiddenCount -= messages.length
      continue
    }

    if (remainingHiddenCount > 0) {
      let visibleStartIndex = remainingHiddenCount
      while (
        visibleStartIndex < messages.length &&
        isWorkflowTaskBoundaryMessage(messages[visibleStartIndex]) &&
        getWorkflowMessageWindowAnchorId(messages[visibleStartIndex]) !== normalizedAnchorId
      ) {
        visibleStartIndex += 1
      }
      visibleGroups.push({
        ...group,
        messages: messages.slice(visibleStartIndex)
      })
      remainingHiddenCount = 0
      continue
    }

    visibleGroups.push(group)
  }

  const visibleMessageTotal = visibleGroups.reduce(
    (total, group) => total + (group?.messages?.length || 0),
    0
  )
  return {
    groups: visibleGroups,
    hiddenMessageCount: Math.max(0, totalMessageCount - visibleMessageTotal)
  }
}

export const getWorkflowPersistedMessageId = message => {
  const value = message?.id
  if (value === null || value === undefined || value === '') return null
  const normalized = String(value).trim()
  return /^\d+$/.test(normalized) ? normalized : null
}

export const getWorkflowPersistedMessageMergeKey = message => {
  const persistedMessageId = getWorkflowPersistedMessageId(message)
  if (!persistedMessageId) return null
  return `${persistedMessageId}:${getWorkflowMessageIdentityDiscriminator(message)}`
}

/**
 * Page responses, snapshots, and live events can overlap while a history request
 * is in flight. Keep one copy of each persisted database row, preferring the
 * later source so current state replaces an older page projection.
 */
export const mergeWorkflowMessagePages = (earlierMessages = [], currentMessages = []) => {
  const merged = []
  const persistedMessageIndex = new Map()

  for (const message of [...earlierMessages, ...currentMessages]) {
    const persistedMessageKey = getWorkflowPersistedMessageMergeKey(message)
    if (!persistedMessageKey) {
      merged.push(message)
      continue
    }

    const existingIndex = persistedMessageIndex.get(persistedMessageKey)
    if (existingIndex === undefined) {
      persistedMessageIndex.set(persistedMessageKey, merged.length)
      merged.push(message)
    } else {
      merged[existingIndex] = message
    }
  }

  return merged
}

/**
 * A queued user message has one canonical queue identifier from the backend.
 * A projection must not render more than one copy when completion-boundary
 * reconciliation temporarily includes the same message in multiple groups.
 */
export const dedupeQueuedUserMessageProjection = (messages = []) => {
  const selectedMessageByQueueId = new Map()

  const getPriority = message => {
    const queueStatus = String(message?.metadata?.queue_status || '').toLowerCase()
    if (queueStatus === 'applied') return 2
    if (message?.id !== null && message?.id !== undefined) return 1
    return 0
  }

  for (const message of messages) {
    const queuedMessageId = String(message?.metadata?.queued_user_message_id || '').trim()
    if (!queuedMessageId || message?.role !== 'user') continue

    const existing = selectedMessageByQueueId.get(queuedMessageId)
    if (!existing || getPriority(message) > getPriority(existing)) {
      selectedMessageByQueueId.set(queuedMessageId, message)
    }
  }

  return messages.filter(message => {
    const queuedMessageId = String(message?.metadata?.queued_user_message_id || '').trim()
    return (
      !queuedMessageId ||
      message?.role !== 'user' ||
      selectedMessageByQueueId.get(queuedMessageId) === message
    )
  })
}

export const resolveAskUserResponse = (
  message,
  responsesByToolCallId = new Map(),
  legacyResponsesBySourceOrder = new Map()
) => {
  if (getStructuredWorkflowToolName(message) !== 'ask_user') return ''

  const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
  if (toolCallId) return responsesByToolCallId.get(toolCallId) || ''

  return legacyResponsesBySourceOrder.get(message?.sourceOrder) || ''
}

export const reconcileWorkflowTaskWindowState = ({
  messages = [],
  workflowId = null,
  state,
  acceptedCompletionIds,
  isAcceptedCompletionMessage,
  buildTaskGroups,
  buildGroupId,
  getMessageIdentity,
  getMessageToolCallId
}) => {
  const emptyState = {
    workflowId,
    initialized: false,
    completedGroups: [],
    activeMessages: [],
    lastCompletionIndex: -1,
    lastCompletionId: '',
    lastCompletionToolCallId: ''
  }

  if (!messages.length) return emptyState

  const initialize = () => {
    acceptedCompletionIds.clear()
    for (const message of messages) {
      if (!isAcceptedCompletionMessage(message)) continue
      const toolCallId = getMessageToolCallId(message)
      if (toolCallId) acceptedCompletionIds.add(toolCallId)
    }

    const groups = buildTaskGroups(messages, true).filter(group =>
      group.messages.some(message => !isWorkflowTaskBoundaryMessage(message))
    )
    const completedGroups = groups.filter(group => group.isCompleted)
    const activeGroup = groups.find(group => !group.isCompleted)
    let lastCompletionIndex = -1

    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index]
      if (
        isWorkflowManualClearContextMessage(message) ||
        acceptedCompletionIds.has(getMessageToolCallId(message)) ||
        isAcceptedCompletionMessage(message)
      ) {
        lastCompletionIndex = index
        break
      }
    }

    return {
      workflowId,
      initialized: true,
      completedGroups,
      activeMessages: activeGroup?.messages || [],
      lastCompletionIndex,
      lastCompletionId:
        lastCompletionIndex >= 0
          ? getMessageIdentity(messages[lastCompletionIndex], lastCompletionIndex)
          : '',
      lastCompletionToolCallId:
        lastCompletionIndex >= 0 ? getMessageToolCallId(messages[lastCompletionIndex]) : ''
    }
  }

  if (!state?.initialized || state.workflowId !== workflowId) return initialize()

  const findCompletionBoundaryIndex = () => {
    if (state.lastCompletionIndex < 0) return -1

    const previousToolCallId = String(state.lastCompletionToolCallId || '')
    if (previousToolCallId) {
      for (let index = messages.length - 1; index >= 0; index -= 1) {
        if (getMessageToolCallId(messages[index]) === previousToolCallId) return index
      }
    }

    const previousIdentity = String(state.lastCompletionId || '')
    if (!previousIdentity) return -1

    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (getMessageIdentity(messages[index], index) === previousIdentity) return index
    }

    return -1
  }

  let lastCompletionIndex = state.lastCompletionIndex
  let lastCompletionId = state.lastCompletionId
  let lastCompletionToolCallId = state.lastCompletionToolCallId

  if (lastCompletionIndex >= 0) {
    const boundaryMessage = messages[lastCompletionIndex]
    if (
      !boundaryMessage ||
      getMessageIdentity(boundaryMessage, lastCompletionIndex) !== lastCompletionId
    ) {
      const relocatedBoundaryIndex = findCompletionBoundaryIndex()
      if (relocatedBoundaryIndex < 0 || relocatedBoundaryIndex !== lastCompletionIndex) {
        return initialize()
      }

      lastCompletionIndex = relocatedBoundaryIndex
      lastCompletionId = getMessageIdentity(messages[relocatedBoundaryIndex], relocatedBoundaryIndex)
      lastCompletionToolCallId = getMessageToolCallId(messages[relocatedBoundaryIndex])
    }
  }

  const activeStartIndex = lastCompletionIndex + 1
  const activeTail = messages.slice(activeStartIndex)
  const tailGroups = buildTaskGroups(activeTail)
  const newlyCompletedGroups = tailGroups.filter(group => group.isCompleted)
  const reconciledGroups = mergeManualClearContextMarkersIntoPreviousGroups(
    [...state.completedGroups, ...tailGroups],
    buildGroupId
  ).filter(group => group.messages.some(message => !isWorkflowTaskBoundaryMessage(message)))
  const completedGroups = reconciledGroups.filter(group => group.isCompleted)
  const activeGroup = reconciledGroups.find(group => !group.isCompleted)

  if (!newlyCompletedGroups.length) {
    return {
      ...state,
      workflowId,
      initialized: true,
      completedGroups,
      activeMessages: activeGroup?.messages || [],
      lastCompletionIndex,
      lastCompletionId,
      lastCompletionToolCallId
    }
  }

  let nextLastCompletionIndex = lastCompletionIndex
  for (let index = activeStartIndex; index < messages.length; index += 1) {
    if (
      isWorkflowManualClearContextMessage(messages[index]) ||
      acceptedCompletionIds.has(getMessageToolCallId(messages[index]))
    ) {
      nextLastCompletionIndex = index
    }
  }

  return {
    workflowId,
    initialized: true,
    completedGroups,
    activeMessages: activeGroup?.messages || [],
    lastCompletionIndex: nextLastCompletionIndex,
    lastCompletionId: getMessageIdentity(messages[nextLastCompletionIndex], nextLastCompletionIndex),
    lastCompletionToolCallId: getMessageToolCallId(messages[nextLastCompletionIndex])
  }
}

export const resolveFinalReviewSubAgentCompletion = (message, completion) => {
  if (completion) return completion

  const metadata = message?.metadata || {}
  const taskId = metadata.sub_agent_id || metadata.data?.sub_agent_id || ''
  const reviewDisplayState = String(metadata.review_display_state || '').toLowerCase()
  if (!taskId || reviewDisplayState !== 'final_review_completed') {
    return null
  }

  const result =
    metadata.review_result && typeof metadata.review_result === 'object'
      ? metadata.review_result
      : {
          status: metadata.sub_agent_status || metadata.execution_status || 'completed',
          result: metadata.review_verdict || metadata.review_summary || '',
          usage_summary: metadata.review_usage_summary
        }

  return {
    execution_status: result.status || metadata.sub_agent_status || metadata.execution_status || '',
    result,
    sub_agent_name: metadata.sub_agent_name || '',
    sub_agent_task: metadata.sub_agent_task || '',
    data: {}
  }
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

// Flexbox `order` accepts integers only. Rank the shared durable order axis so
// pending and persisted tools keep the same interleaving without passing fractions to CSS.
export const getWorkflowToolGroupRenderOrders = (thoughts = [], tools = []) => {
  const entries = [
    ...thoughts.map((item, index) => ({
      kind: 'thought',
      index,
      sourceIndex: index,
      order:
        item?.groupOrder !== null &&
        item?.groupOrder !== undefined &&
        Number.isFinite(Number(item.groupOrder))
          ? Number(item.groupOrder)
          : index
    })),
    ...tools.map((item, index) => ({
      kind: 'tool',
      index,
      sourceIndex: thoughts.length + index,
      order:
        item?.groupOrder !== null &&
        item?.groupOrder !== undefined &&
        Number.isFinite(Number(item.groupOrder))
          ? Number(item.groupOrder)
          : index
    }))
  ].sort((left, right) => left.order - right.order || left.sourceIndex - right.sourceIndex)

  const thoughtOrders = Array(thoughts.length)
  const toolOrders = Array(tools.length)
  entries.forEach((entry, renderOrder) => {
    if (entry.kind === 'thought') {
      thoughtOrders[entry.index] = renderOrder
    } else {
      toolOrders[entry.index] = renderOrder
    }
  })

  return { thoughtOrders, toolOrders }
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

const WORKFLOW_TOOL_RUNNING_DISPLAY_STATUSES = new Set([
  'running',
  'pending',
  'queued',
  'waiting',
  'awaiting_execution',
  'approval_submitted'
])

/**
 * Waiting for an ask_user response is a user-input wait, not tool execution.
 * Keep its durable execution_status intact while avoiding the running animation.
 */
export const isWorkflowToolRunningForDisplay = (message, approvedSubmission = false) => {
  const executionStatus = String(message?.metadata?.execution_status || '').toLowerCase()
  if (executionStatus === 'waiting' && getStructuredWorkflowToolName(message) === 'ask_user') {
    return false
  }
  if (WORKFLOW_TOOL_RUNNING_DISPLAY_STATUSES.has(executionStatus)) return true
  return isWorkflowToolAwaitingExecution(message, approvedSubmission)
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

const getWorkflowToolCallId = value =>
  String(value?.id || value?.tool_call_id || value?.toolCallId || '').trim()

const isHiddenAskUserResponse = message =>
  message?.role === 'user' &&
  (message?.metadata?.ui_visibility || message?.metadata?.uiVisibility) === 'hide' &&
  /<ask_user_response>\s*[\s\S]*?<\/ask_user_response>/i.test(message?.message || '')

/**
 * A page can begin mid-tool-chain after the raw 301-row read. Keep loading
 * within the backend-selected task window until every rendered observation has
 * its assistant declaration and every hidden ask_user answer has its source
 * tool record. These dependencies are not independent display cards.
 */
export const hasIncompleteWorkflowToolCallChain = (messages = []) => {
  const declaredToolCallIds = new Set()
  const observedToolCallIds = new Set()
  const askUserToolCallIds = new Set()
  const askUserResponseToolCallIds = new Set()

  for (const message of messages) {
    if (message?.role === 'assistant') {
      for (const call of message?.metadata?.tool_calls || []) {
        const toolCallId = getWorkflowToolCallId(call)
        if (toolCallId) declaredToolCallIds.add(toolCallId)
      }
    }

    const toolCallId = String(message?.metadata?.tool_call_id || '').trim()
    const isToolObservation =
      message?.role === 'tool' ||
      (message?.role === 'user' && String(message?.stepType || '').toLowerCase() === 'observe')
    if (toolCallId && isToolObservation) observedToolCallIds.add(toolCallId)

    if (message?.role === 'tool' && getStructuredWorkflowToolName(message) === 'ask_user') {
      if (toolCallId) askUserToolCallIds.add(toolCallId)
      continue
    }

    if (isHiddenAskUserResponse(message)) {
      if (!toolCallId) return true
      askUserResponseToolCallIds.add(toolCallId)
    }
  }

  return (
    [...observedToolCallIds].some(toolCallId => !declaredToolCallIds.has(toolCallId)) ||
    [...askUserResponseToolCallIds].some(toolCallId => !askUserToolCallIds.has(toolCallId))
  )
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

export const isWorkflowContextSnapshotMessage = message =>
  message?.role === 'system' &&
  message?.messageKind === 'summary' &&
  !isWorkflowManualClearContextMessage(message)

export const isWorkflowExplorationBatchMessage = message =>
  message?.metadata?.message_kind === 'exploration_batch'

export const isCollapsedWorkflowToolGroupMessage = message =>
  message?.metadata?.message_kind === 'tool_group'

export const getWorkflowAskUserResponseItems = message => {
  const content = message?.askUserResponse || message?.message || ''
  const match = content.match(/<ask_user_response>\s*([\s\S]*?)\s*<\/ask_user_response>/i)
  if (!match) return []

  try {
    const parsed = JSON.parse(match[1])
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

const COLLAPSIBLE_READ_TOOL_NAMES = new Set(['read_file', 'list_dir', 'web_fetch'])
const COLLAPSIBLE_SEARCH_TOOL_NAMES = new Set(['grep', 'glob', 'web_search'])
const COLLAPSIBLE_COMMAND_TOOL_NAMES = new Set(['bash'])
const COLLAPSIBLE_MUTATION_TOOL_NAMES = new Set(['edit_file', 'write_file'])
const NON_COLLAPSIBLE_TOOL_NAMES = new Set([
  'ask_user',
  'submit_plan',
  'complete_workflow',
  'sub_agent_run',
  'sub_agent_stop'
])
const TODO_TOOL_NAMES = new Set(['todo_create', 'todo_list', 'todo_update', 'todo_get'])
const TOOL_GROUP_LABEL_KEYS = {
  bash: 'workflow.toolGroups.runCommand',
  edit_file: 'workflow.toolGroups.editFile',
  glob: 'workflow.toolGroups.fileSearch',
  grep: 'workflow.toolGroups.fileSearch',
  list_dir: 'workflow.toolGroups.readFile',
  read_file: 'workflow.toolGroups.readFile',
  skill: 'workflow.toolGroups.useSkill',
  sub_agent_output: 'workflow.toolGroups.fetchSubAgentResult',
  web_fetch: 'workflow.toolGroups.readWeb',
  web_search: 'workflow.toolGroups.fileSearch',
  write_file: 'workflow.toolGroups.editFile'
}
const TOOL_GROUP_ICONS = {
  command_tools: 'bash',
  mcp_tools: 'mcp',
  mutation_tools: 'edit',
  readonly_tools: 'search',
  sub_agent_result_tools: 'skill-relation-chart',
  todo_tools: 'todo'
}

const defaultRemoveSystemReminder = value => String(value || '').trimEnd()
const defaultTranslate = key => key

/**
 * Build the list-specific semantic projection from durable UI messages.
 *
 * This is deliberately independent from Vue state: current pending approvals,
 * localization and system-reminder cleanup enter through explicit adapters.
 * The component may render, expand and scroll this projection, but must not
 * recreate its grouping or visibility rules.
 */
export const projectWorkflowMessageList = (
  messages = [],
  {
    pendingApprovalIds = [],
    removeSystemReminder = defaultRemoveSystemReminder,
    translate = defaultTranslate
  } = {}
) => {
  const sourceMessages = Array.isArray(messages) ? messages : []
  const isApprovalPending = message =>
    isWorkflowMessagePendingApproval(message, pendingApprovalIds)
  const getToolName = message => getStructuredWorkflowToolName(message)
  const getToolCategory = message =>
    message?.metadata?.tool_category || message?.metadata?.toolCategory || null
  const isFinishTaskErrorMessage = message =>
    !!(
      message &&
      message.role === 'tool' &&
      isWorkflowCompletionMessage(message) &&
      message.toolDisplay?.isError
    )
  const isSameFinishTaskError = (left, right) =>
    isFinishTaskErrorMessage(left) &&
    isFinishTaskErrorMessage(right) &&
    removeSystemReminder(left.message || '') === removeSystemReminder(right.message || '') &&
    (left.toolDisplay?.summary || '') === (right.toolDisplay?.summary || '')
  const isCompletionReportMessage = message =>
    message?.role === 'assistant' && message?.metadata?.message_kind === 'completion_report'
  const isThinkOnlyAssistantMessage = message =>
    message?.role === 'assistant' &&
    !hasVisibleWorkflowText(removeSystemReminder(message?.message || '')) &&
    hasVisibleWorkflowText(message?.reasoning || '')
  const getReadOnlyToolCategory = toolName => {
    if (COLLAPSIBLE_READ_TOOL_NAMES.has(toolName)) return 'read'
    if (COLLAPSIBLE_SEARCH_TOOL_NAMES.has(toolName)) return 'search'
    return null
  }
  const isCollapsibleToolMessage = message => {
    if (message?.role !== 'tool') return false
    const toolName = getToolName(message)
    return !!toolName && !NON_COLLAPSIBLE_TOOL_NAMES.has(toolName) && !isApprovalPending(message)
  }
  const isCollapsibleReadOnlyToolMessage = message => {
    const toolName = getToolName(message)
    return (
      isCollapsibleToolMessage(message) &&
      !TODO_TOOL_NAMES.has(toolName) &&
      getReadOnlyToolCategory(toolName) !== null
    )
  }
  const isCollapsibleTodoToolMessage = message =>
    isCollapsibleToolMessage(message) && TODO_TOOL_NAMES.has(getToolName(message))
  const isCollapsibleCommandToolMessage = message =>
    isCollapsibleToolMessage(message) &&
    COLLAPSIBLE_COMMAND_TOOL_NAMES.has(getToolName(message))
  const isCollapsibleMutationToolMessage = message =>
    isCollapsibleToolMessage(message) &&
    COLLAPSIBLE_MUTATION_TOOL_NAMES.has(getToolName(message))
  const isCollapsibleMcpToolMessage = message =>
    isCollapsibleToolMessage(message) &&
    isWorkflowMcpTool(getToolName(message), getToolCategory(message))
  const isCollapsibleSubAgentResultToolMessage = message =>
    isCollapsibleToolMessage(message) && getToolName(message) === 'sub_agent_output'
  const getCollapsibleToolGroupKind = message => {
    if (isCollapsibleReadOnlyToolMessage(message)) return 'readonly_tools'
    if (isCollapsibleTodoToolMessage(message)) return 'todo_tools'
    if (isCollapsibleCommandToolMessage(message)) return 'command_tools'
    if (isCollapsibleMutationToolMessage(message)) return 'mutation_tools'
    if (isCollapsibleSubAgentResultToolMessage(message)) return 'sub_agent_result_tools'
    if (isCollapsibleMcpToolMessage(message)) return 'mcp_tools'
    return isCollapsibleToolMessage(message) ? 'other_tools' : null
  }
  const getToolCallId = call => String(call?.id || call?.tool_call_id || '').trim()
  const getToolGroupLabel = message => {
    const toolName = getToolName(message)
    if (TODO_TOOL_NAMES.has(toolName)) return translate('workflow.toolGroups.taskChanges')
    if (isWorkflowMcpTool(toolName, getToolCategory(message))) return translate('workflow.toolGroups.callMcp')
    return translate(TOOL_GROUP_LABEL_KEYS[toolName] || 'workflow.toolGroups.useTool')
  }
  const truncateToolGroupText = (value, maxLength = 48) => {
    const text = String(value || '')
      .replace(/\s+/g, ' ')
      .trim()
    if (!text) return ''
    return text.length > maxLength ? `${text.slice(0, maxLength - 3)}...` : text
  }
  const buildToolGroupSummary = groupMessages => {
    const counts = new Map()
    groupMessages.forEach(message => {
      const label = getToolGroupLabel(message)
      if (label) counts.set(label, (counts.get(label) || 0) + 1)
    })
    return truncateToolGroupText(
      Array.from(counts, ([label, count]) => `${label} x${count}`).join(' · '),
      120
    )
  }
  const getCollapsedToolGroupExpandId = (groupMessages, index) => {
    const first = groupMessages[0]
    const firstId = first?.displayId || first?.id || `tool_group_${index}`
    const firstToolCallId = String(first?.metadata?.tool_call_id || '').trim()
    return `tool_group:${firstToolCallId || firstId}`
  }
  const getToolGroupIcon = (kind, groupMessages) => {
    if (groupMessages.some(isCollapsibleMutationToolMessage)) return TOOL_GROUP_ICONS.mutation_tools
    return TOOL_GROUP_ICONS[kind] || 'tool'
  }
  const buildToolGroupMessage = (groupMessages, index, thoughts = [], isOngoing = false) => {
    const kinds = new Set(groupMessages.map(getCollapsibleToolGroupKind).filter(Boolean))
    const [singleKind] = kinds
    const kind = groupMessages.length > 0 && kinds.size === 1 ? singleKind : 'mixed_tools'
    const thoughtCount = thoughts.length
    const errorCount = groupMessages.filter(
      message => !!(message?.toolDisplay?.isError || message?.isRejected)
    ).length
    const seedMessage = groupMessages[0] || thoughts[0] || {}
    const firstActivity = [...thoughts, ...groupMessages].reduce((earliest, item) => {
      if (!earliest) return item
      return (item?.groupOrder ?? index) < (earliest?.groupOrder ?? index) ? item : earliest
    }, null)
    const { thoughtOrders, toolOrders } = getWorkflowToolGroupRenderOrders(thoughts, groupMessages)
    const groupedThoughts = thoughts.map((thought, thoughtIndex) => ({
      ...thought,
      renderOrder: thoughtOrders[thoughtIndex]
    }))
    const groupedTools = groupMessages.map((tool, toolIndex) => ({
      ...tool,
      renderOrder: toolOrders[toolIndex]
    }))

    return {
      ...seedMessage,
      role: 'assistant',
      displayId: getCollapsedToolGroupExpandId([firstActivity || seedMessage], index),
      metadata: {
        ...(seedMessage?.metadata || {}),
        message_kind: 'tool_group',
        tool_group_kind: kind,
        tool_group_thought_count: thoughtCount,
        tool_group_is_ongoing: isOngoing
      },
      groupDisplay: {
        icon: getToolGroupIcon(kind, groupMessages),
        thoughtSummary: thoughtCount
          ? translate('workflow.toolGroups.thoughtChangeCount', { count: thoughtCount })
          : '',
        errorSummary: errorCount
          ? translate('workflow.toolGroups.errorCount', { count: errorCount })
          : '',
        summary: buildToolGroupSummary(groupMessages)
      },
      groupedThoughts,
      groupedTools
    }
  }
  const buildPendingToolGroupItem = (call, groupOrder) => {
    const toolCallId = getToolCallId(call)
    const toolName = String(call?.toolName || '').trim()
    const argumentsValue = call?.arguments ?? {}

    return {
      id: `pending_tool:${toolCallId}`,
      groupOrder,
      displayId: `pending_tool:${toolCallId}`,
      role: 'tool',
      message: '',
      isRejected: !!call?.isRejected,
      isApproved: false,
      metadata: {
        tool_call_id: toolCallId,
        tool_name: toolName,
        tool_call: {
          id: toolCallId,
          function: {
            name: toolName,
            arguments: argumentsValue
          }
        },
        approval_status: 'approved',
        execution_status: 'running'
      },
      toolDisplay: {
        icon: call?.icon || 'tool',
        toolType: call?.toolType || 'tool-system',
        action: call?.action || toolName,
        target: call?.target || '',
        summary: call?.summary || '',
        isError: !!call?.isRejected
      }
    }
  }
  const projectPendingToolGroups = input => {
    const projected = []
    input.forEach((message, index) => {
      const pendingCalls = Array.isArray(message?.pendingToolCalls) ? message.pendingToolCalls : []
      const groupedTools = pendingCalls
        .map((call, callIndex) =>
          buildPendingToolGroupItem(
            call,
            call.groupOrder ?? index + (callIndex + 1) / (pendingCalls.length + 1)
          )
        )
        .filter(tool => getCollapsibleToolGroupKind(tool))
      if (groupedTools.length === 0) {
        projected.push(message)
        return
      }

      const groupedToolIds = new Set(groupedTools.map(tool => tool.metadata.tool_call_id))
      const remainingPendingCalls = pendingCalls.filter(
        call => !groupedToolIds.has(getToolCallId(call))
      )
      const hasAssistantContent = Boolean(
        hasVisibleWorkflowText(removeSystemReminder(message?.message || '')) ||
          hasVisibleWorkflowText(message?.reasoning || '')
      )
      if (message?.role !== 'assistant' || hasAssistantContent || remainingPendingCalls.length > 0) {
        projected.push({ ...message, pendingToolCalls: remainingPendingCalls })
      }
      projected.push(buildToolGroupMessage(groupedTools, index, [], true))
    })
    return projected
  }
  const isToolGroupBoundaryMessage = message => {
    if (!message) return true
    if (isCollapsedWorkflowToolGroupMessage(message)) return false
    if (message.role === 'user') return true
    if (isWorkflowContextSnapshotMessage(message) || isWorkflowManualClearContextMessage(message)) {
      return true
    }
    if (isCompletionReportMessage(message) || isWorkflowCompletionMessage(message)) return true
    if (isWorkflowExplorationBatchMessage(message)) return true
    if (message.role === 'assistant') {
      return (
        !isThinkOnlyAssistantMessage(message) &&
        hasVisibleWorkflowText(removeSystemReminder(message?.message || ''))
      )
    }
    return message.role === 'tool' && !getCollapsibleToolGroupKind(message)
  }
  const isToolGroupOngoingBoundaryMessage = message => {
    if (!message || isCollapsedWorkflowToolGroupMessage(message)) return false
    if (message.role === 'user') return true
    if (isWorkflowContextSnapshotMessage(message) || isWorkflowManualClearContextMessage(message)) {
      return true
    }
    if (isCompletionReportMessage(message) || isWorkflowCompletionMessage(message)) return true
    if (isWorkflowExplorationBatchMessage(message)) return true
    if (message.role === 'assistant') {
      return (
        !isThinkOnlyAssistantMessage(message) &&
        hasVisibleWorkflowText(removeSystemReminder(message?.message || ''))
      )
    }
    return message.role === 'tool' && (isApprovalPending(message) || !getCollapsibleToolGroupKind(message))
  }
  const hasOngoingToolGroupAfter = (input, startIndex) => {
    for (let index = startIndex; index < input.length; index += 1) {
      if (isToolGroupOngoingBoundaryMessage(input[index])) return false
    }
    return true
  }
  const isStandaloneOngoingThoughtRun = (input, startIndex) => {
    for (let index = startIndex; index < input.length; index += 1) {
      const message = input[index]
      if (isToolGroupOngoingBoundaryMessage(message)) return false
      if (getCollapsibleToolGroupKind(message) || isCollapsedWorkflowToolGroupMessage(message)) {
        return false
      }
      if (!isThinkOnlyAssistantMessage(message)) return false
    }
    return true
  }
  const buildGroupedThoughtItem = (message, index, order = index) => ({
    ...message,
    displayId: `${message?.displayId || message?.id || `thought_${index}`}:tool_group_thought`,
    groupOrder: order,
    sourceMessage: message
  })
  const collectContiguousToolMessages = (input, startIndex) => {
    const tools = []
    let nextIndex = startIndex
    while (nextIndex < input.length && getCollapsibleToolGroupKind(input[nextIndex])) {
      tools.push(input[nextIndex])
      nextIndex += 1
    }
    return { tools, nextIndex }
  }
  const collectToolActivityMessage = (message, index, thoughts, tools) => {
    if (isThinkOnlyAssistantMessage(message)) {
      thoughts.push(buildGroupedThoughtItem(message, index, message.groupOrder ?? index))
      return
    }
    if (isCollapsedWorkflowToolGroupMessage(message)) {
      ;(message.groupedThoughts || []).forEach((thought, thoughtIndex) => {
        thoughts.push(
          buildGroupedThoughtItem(
            thought,
            `${index}_${thoughtIndex}`,
            thought.groupOrder ?? index + thoughtIndex / 1000
          )
        )
      })
      ;(message.groupedTools || []).forEach((tool, toolIndex) => {
        if (getCollapsibleToolGroupKind(tool)) {
          tools.push({ ...tool, groupOrder: tool.groupOrder ?? index + (toolIndex + 1) / 1000 })
        }
      })
      return
    }
    if (getCollapsibleToolGroupKind(message)) {
      tools.push({ ...message, groupOrder: message.groupOrder ?? index })
    }
  }
  const collapseToolActivityGroups = input => {
    const collapsed = []
    for (let index = 0; index < input.length; ) {
      const current = input[index]
      const currentIsThought = isThinkOnlyAssistantMessage(current)
      const currentToolGroupKind = getCollapsibleToolGroupKind(current)
      if (currentIsThought || currentToolGroupKind || isCollapsedWorkflowToolGroupMessage(current)) {
        if (currentToolGroupKind) {
          const contiguousGroup = collectContiguousToolMessages(input, index)
          const nextMessage = input[contiguousGroup.nextIndex]
          if (!isThinkOnlyAssistantMessage(nextMessage) && !isCollapsedWorkflowToolGroupMessage(nextMessage)) {
            collapsed.push(
              buildToolGroupMessage(
                contiguousGroup.tools,
                index,
                [],
                hasOngoingToolGroupAfter(input, contiguousGroup.nextIndex)
              )
            )
            index = contiguousGroup.nextIndex
            continue
          }
        }

        const thoughts = []
        const tools = []
        let nextIndex = index
        if (currentIsThought && isStandaloneOngoingThoughtRun(input, index)) {
          while (
            nextIndex < input.length &&
            isThinkOnlyAssistantMessage(input[nextIndex]) &&
            isStandaloneOngoingThoughtRun(input, nextIndex)
          ) {
            collectToolActivityMessage(input[nextIndex], nextIndex, thoughts, tools)
            nextIndex += 1
          }
          collapsed.push(buildToolGroupMessage([], index, thoughts, true))
          index = nextIndex
          continue
        }

        while (nextIndex < input.length && !isToolGroupBoundaryMessage(input[nextIndex])) {
          collectToolActivityMessage(input[nextIndex], nextIndex, thoughts, tools)
          nextIndex += 1
        }
        if (tools.length > 0) {
          collapsed.push(
            buildToolGroupMessage(tools, index, thoughts, hasOngoingToolGroupAfter(input, nextIndex))
          )
          index = nextIndex
          continue
        }
      }
      collapsed.push(current)
      index += 1
    }
    return collapsed
  }
  const collapseRepeatedFinishTaskErrors = input => {
    const collapsed = []
    for (let index = 0; index < input.length; ) {
      const current = input[index]
      if (!isFinishTaskErrorMessage(current)) {
        collapsed.push(current)
        index += 1
        continue
      }
      let count = 1
      let nextIndex = index + 1
      while (nextIndex < input.length && isSameFinishTaskError(current, input[nextIndex])) {
        count += 1
        nextIndex += 1
      }
      collapsed.push(
        count > 1
          ? {
              ...current,
              displayId: `${current.displayId || current.id || `finish_task_${index}`}_collapsed_${count}`,
              metadata: { ...(current.metadata || {}), finish_task_error_count: count }
            }
          : current
      )
      index = nextIndex
    }
    return collapsed
  }
  const collapseAssistantCompletionPairs = input =>
    input.filter(
      (message, index) =>
        !(
          isThinkOnlyAssistantMessage(message) &&
          isCompletionReportMessage(input[index + 1]) &&
          String(message.stepIndex || '') === String(input[index + 1].stepIndex || '')
        )
    )
  const isHiddenSystemObservation = message => {
    if (message?.metadata?.ui_visibility === 'hide') return true
    if (message?.metadata?.message_kind === 'runtime_observation') return false
    if (message?.metadata?.error_type === 'SubAgentInterrupted') return true
    if (message?.role !== 'user' || String(message.stepType || '').toLowerCase() !== 'observe') {
      return false
    }
    if (getWorkflowAskUserResponseItems(message).length > 0) return false
    return !hasVisibleWorkflowText(removeSystemReminder(message.message || ''))
  }

  const isEmptyAssistantMessage = message =>
    message?.role === 'assistant' &&
    !isCollapsedWorkflowToolGroupMessage(message) &&
    !isCompletionReportMessage(message) &&
    !hasVisibleWorkflowText(removeSystemReminder(message?.message || '')) &&
    !hasVisibleWorkflowText(message?.reasoning || '') &&
    !(message?.pendingToolCalls?.length > 0)

  return excludeLeadingManualClearContextMarkers(
    collapseToolActivityGroups(
      projectPendingToolGroups(
        collapseAssistantCompletionPairs(
          collapseRepeatedFinishTaskErrors(
            sourceMessages.filter(
              message =>
                (!isHiddenSystemObservation(message) || isWorkflowManualClearContextMessage(message)) &&
                !isEmptyAssistantMessage(message)
            )
          )
        )
      )
    )
  )
}
