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
export const selectVisibleWorkflowMessageWindow = (groups = [], visibleMessageCount = 200) => {
  const normalizedLimit = Math.max(0, Math.floor(Number(visibleMessageCount) || 0))
  const totalMessageCount = groups.reduce(
    (total, group) => total + (group?.messages?.length || 0),
    0
  )
  let remainingHiddenCount = Math.max(0, totalMessageCount - normalizedLimit)
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
        isWorkflowTaskBoundaryMessage(messages[visibleStartIndex])
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
