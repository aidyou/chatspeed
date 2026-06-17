import { ref, computed, watch } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
import { useSubAgentSummaries } from './useSubAgentSummaries'
import { resolveWorkflowToolIcon } from './toolIcons'
import { isAutoExecuteWorkflowTool } from './toolApproval'
import { useI18n } from 'vue-i18n'
import * as Diff from 'diff'
import {
  formatDisplayPath,
  getToolStatusSummary,
  normalizeShellCommandForDisplay,
  normalizeToolDisplayText
} from './toolDisplay'

/**
 * Composable for managing message processing and display
 * Handles enhanced messages, tool formatting, and expansion states
 */
const DEFAULT_VISIBLE_COMPLETED_TASK_GROUPS = 3

export function useWorkflowMessages(options = {}) {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()
  const visibleCompletedTaskGroupCount =
    options.visibleCompletedTaskGroupCount || ref(DEFAULT_VISIBLE_COMPLETED_TASK_GROUPS)

  const expandedMessages = ref(new Set())
  const expandedReasonings = ref(new Set())
  const taskGroupCache = new Map()
  const acceptedTaskCompletionIds = new Set()
  const taskWindowState = ref({
    workflowId: null,
    initialized: false,
    completedGroups: [],
    activeMessages: [],
    lastCompletionIndex: -1,
    lastCompletionId: '',
    lastCompletionToolCallId: ''
  })

  const removeSystemReminder = content => {
    if (content === null || content === undefined) return ''
    return String(content)
      .replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '')
      .trimEnd()
  }

  const isHiddenSystemObservation = message => {
    const uiVisibility = message?.metadata?.ui_visibility || message?.metadata?.uiVisibility
    if (uiVisibility === 'hide') return true
    if (message?.role !== 'user') return false
    if ((message.stepType || '').toLowerCase() !== 'observe') return false
    return removeSystemReminder(message.message || '').trim() === ''
  }

  const isSubAgentCompletionObservation = message => {
    const meta = message?.metadata || {}
    return meta?.observation_type === 'sub_agent_completion'
  }

  const isFinishTaskMessage = message => {
    const toolName = String(
      message?.metadata?.tool_name ||
        message?.metadata?.tool_call?.name ||
        message?.metadata?.tool_call?.function?.name ||
        ''
    ).toLowerCase()
    return toolName === 'complete_workflow_with_summary'
  }

  const getMessageToolCallId = message =>
    String(message?.metadata?.tool_call_id || message?.metadata?.toolCallId || '')

  const isAcceptedFinishTaskMessage = message => {
    if (message?.role !== 'tool' || !isFinishTaskMessage(message)) return false

    const metadata = message.metadata || {}
    const executionStatus = String(metadata.execution_status || '').toLowerCase()
    const approvalStatus = String(metadata.approval_status || '').toLowerCase()
    const reviewDisplayState = String(metadata.review_display_state || '').toLowerCase()
    const isError = Boolean(
      message.isError ||
        message.is_error ||
        metadata.is_error ||
        message.errorType ||
        message.error_type ||
        metadata.error_type ||
        metadata.errorType
    )

    if (isError || approvalStatus === 'rejected' || reviewDisplayState === 'final_review_rejected') {
      return false
    }
    if (reviewDisplayState === 'final_review_approved') return executionStatus === 'completed'
    return executionStatus ? executionStatus === 'completed' : true
  }

  const filteredWorkflowMessages = computed(() =>
    (workflowStore.messages || []).filter(message => {
      if (isHiddenSystemObservation(message) && !isSubAgentCompletionObservation(message)) {
        return false
      }
      const messageWorkflowId = message?.sessionId || message?.session_id
      return !messageWorkflowId || messageWorkflowId === workflowStore.currentWorkflowId
    })
  )

  const { childAgentSummariesAll } = useSubAgentSummaries()
  const childAgentSummariesRevision = computed(() =>
    childAgentSummariesAll.value
      .map(summary => `${summary.id}:${summary.contextPercent ?? 'null'}:${summary.toolCalls}:${summary.status}`)
      .join('|')
  )

  const childAgentSummaryById = computed(
    () => new Map(childAgentSummariesAll.value.map(summary => [summary.id, summary]))
  )

  const getMessageSegmentId = message => {
    const rawSegmentId = message?.segment_id ?? message?.segmentId ?? message?.metadata?.segment_id
    const parsed = Number(rawSegmentId)
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null
  }

  const pickPreferredNumber = (...values) => {
    for (const value of values) {
      if (typeof value === 'number' && Number.isFinite(value) && value > 0) return value
    }
    for (const value of values) {
      if (typeof value === 'number' && Number.isFinite(value) && value >= 0) return value
    }
    return null
  }

  const pickPreferredText = (...values) => {
    for (const value of values) {
      if (typeof value === 'string' && value.trim()) return value
    }
    return ''
  }

  const buildTaskGroupId = messages => {
    const workflowId = workflowStore.currentWorkflowId || 'workflow'
    const segmentId = getMessageSegmentId(messages[0])
    if (segmentId !== null && messages.every(message => getMessageSegmentId(message) === segmentId)) {
      return `${workflowId}:segment:${segmentId}`
    }

    const first = messages[0]
    const last = messages[messages.length - 1]
    const firstId = first?.id || first?.displayId || `${first?.role || 'msg'}_${first?.stepIndex || 0}`
    const lastId = last?.id || last?.displayId || `${last?.role || 'msg'}_${last?.stepIndex || 0}`
    return `${workflowId}:${firstId}:${lastId}:${messages.length}`
  }

  const buildTaskGroupSignature = messages =>
    messages
      .map(message => {
        const meta = message?.metadata || {}
        const toolCalls = Array.isArray(meta.tool_calls) ? meta.tool_calls.length : 0
        return [
          message?.id || '',
          message?.role || '',
          message?.stepType || '',
          message?.stepIndex || '',
          message?.message || '',
          message?.reasoning || '',
          meta.tool_call_id || '',
          meta.execution_status || '',
          meta.approval_status || '',
          meta.title || '',
          meta.summary || '',
          meta.message_kind || meta.messageKind || '',
          toolCalls,
          message?.isError || message?.is_error ? '1' : '0'
        ].join('::')
      })
      .join('||')

  const buildEnhancedMessageSignature = message =>
    JSON.stringify({
      displayId: message?.displayId || '',
      role: message?.role || '',
      stepType: message?.stepType || '',
      stepIndex: message?.stepIndex || '',
      message: message?.message || '',
      reasoning: message?.reasoning || '',
      metadata: message?.metadata || {},
      isError: !!message?.isError,
      isRejected: !!message?.isRejected,
      isApproved: !!message?.isApproved,
      toolDisplay: message?.toolDisplay || null,
      pendingToolCalls: message?.pendingToolCalls || [],
      subAgentCard: message?.subAgentCard || null
    })

  const reuseUnchangedEnhancedMessages = (cachedEntry, nextMessages) => {
    if (!cachedEntry?.messages?.length) {
      return {
        messages: nextMessages,
        messageSignatures: nextMessages.map(buildEnhancedMessageSignature)
      }
    }

    const cachedById = new Map(
      cachedEntry.messages.map((message, index) => [
        message.displayId,
        {
          message,
          signature: cachedEntry.messageSignatures[index]
        }
      ])
    )
    const messageSignatures = nextMessages.map(buildEnhancedMessageSignature)
    const messages = nextMessages.map((message, index) => {
      const cached = cachedById.get(message.displayId)
      return cached?.signature === messageSignatures[index] ? cached.message : message
    })

    return {
      messages,
      messageSignatures
    }
  }

  const buildTaskGroups = (messages, allowPersistedCompletionFallback = false) => {
    if (!messages.length) return []

    const groups = []
    let currentGroup = []

    const pushGroup = (groupMessages, isCompleted) => {
      if (!groupMessages.length) return
      groups.push({
        id: buildTaskGroupId(groupMessages),
        isCompleted,
        messages: groupMessages
      })
    }

    for (const message of messages) {
      currentGroup.push(message)
      const toolCallId = getMessageToolCallId(message)
      const isAcceptedBoundary =
        acceptedTaskCompletionIds.has(toolCallId) ||
        (allowPersistedCompletionFallback && isAcceptedFinishTaskMessage(message))
      if (isAcceptedBoundary) {
        pushGroup(currentGroup, true)
        currentGroup = []
      }
    }

    pushGroup(currentGroup, false)
    return groups
  }

  const getMessageIdentity = (message, index) =>
    String(
      message?.id ||
        message?.displayId ||
        `${message?.role || 'message'}:${message?.stepIndex || 0}:${index}`
    )

  const initializeTaskWindow = messages => {
    acceptedTaskCompletionIds.clear()
    for (const message of messages) {
      if (isAcceptedFinishTaskMessage(message)) {
        const toolCallId = getMessageToolCallId(message)
        if (toolCallId) acceptedTaskCompletionIds.add(toolCallId)
      }
    }

    const groups = buildTaskGroups(messages, true)
    const completedGroups = groups.filter(group => group.isCompleted)
    const activeGroup = groups.find(group => !group.isCompleted)
    let lastCompletionIndex = -1

    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (acceptedTaskCompletionIds.has(getMessageToolCallId(messages[index]))) {
        lastCompletionIndex = index
        break
      }
    }

    taskWindowState.value = {
      workflowId: workflowStore.currentWorkflowId,
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

  const findCompletionBoundaryIndex = (messages, state) => {
    if (!messages.length || state.lastCompletionIndex < 0) return -1

    const previousToolCallId = String(state.lastCompletionToolCallId || '')
    if (previousToolCallId) {
      for (let index = messages.length - 1; index >= 0; index -= 1) {
        if (getMessageToolCallId(messages[index]) === previousToolCallId) {
          return index
        }
      }
    }

    const previousIdentity = String(state.lastCompletionId || '')
    if (!previousIdentity) return -1

    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (getMessageIdentity(messages[index], index) === previousIdentity) {
        return index
      }
    }

    return -1
  }

  const reconcileTaskWindow = messages => {
    const workflowId = workflowStore.currentWorkflowId
    const previousState = taskWindowState.value

    if (previousState.workflowId !== workflowId) {
      taskGroupCache.clear()
      acceptedTaskCompletionIds.clear()
      taskWindowState.value = {
        workflowId,
        initialized: false,
        completedGroups: [],
        activeMessages: [],
        lastCompletionIndex: -1,
        lastCompletionId: '',
        lastCompletionToolCallId: ''
      }
    }

    if (!messages.length) {
      taskWindowState.value = {
        ...taskWindowState.value,
        initialized: false,
        completedGroups: [],
        activeMessages: [],
        lastCompletionIndex: -1,
        lastCompletionId: '',
        lastCompletionToolCallId: ''
      }
      return
    }

    if (!taskWindowState.value.initialized) {
      initializeTaskWindow(messages)
      return
    }

    const currentState = taskWindowState.value
    let lastCompletionIndex = currentState.lastCompletionIndex
    let lastCompletionId = currentState.lastCompletionId
    let lastCompletionToolCallId = currentState.lastCompletionToolCallId

    if (lastCompletionIndex >= 0) {
      const boundaryMessage = messages[lastCompletionIndex]
      if (!boundaryMessage || getMessageIdentity(boundaryMessage, lastCompletionIndex) !== lastCompletionId) {
        const relocatedBoundaryIndex = findCompletionBoundaryIndex(messages, currentState)
        if (relocatedBoundaryIndex < 0) {
          initializeTaskWindow(messages)
          return
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
    const activeGroup = tailGroups.find(group => !group.isCompleted)

    if (!newlyCompletedGroups.length) {
      taskWindowState.value = {
        ...currentState,
        workflowId,
        initialized: true,
        activeMessages: activeTail,
        lastCompletionIndex,
        lastCompletionId,
        lastCompletionToolCallId
      }
      return
    }

    const completedMessageCount = newlyCompletedGroups.reduce(
      (count, group) => count + group.messages.length,
      0
    )
    const nextLastCompletionIndex = activeStartIndex + completedMessageCount - 1

    taskWindowState.value = {
      workflowId,
      initialized: true,
      completedGroups: [...currentState.completedGroups, ...newlyCompletedGroups],
      activeMessages: activeGroup?.messages || [],
      lastCompletionIndex: nextLastCompletionIndex,
      lastCompletionId: getMessageIdentity(messages[nextLastCompletionIndex], nextLastCompletionIndex),
      lastCompletionToolCallId: getMessageToolCallId(messages[nextLastCompletionIndex])
    }
  }

  watch(
    [
      () => workflowStore.currentWorkflowId,
      filteredWorkflowMessages,
      () => workflowStore.taskCompletionRevision
    ],
    ([, messages]) => {
      const completion = workflowStore.lastTaskCompletion
      if (completion?.sessionId === workflowStore.currentWorkflowId && completion.toolCallId) {
        acceptedTaskCompletionIds.add(completion.toolCallId)
      }
      reconcileTaskWindow(messages)
    },
    { immediate: true, flush: 'sync' }
  )

  const visibleTaskGroupsState = computed(() => {
    const state = taskWindowState.value
    const activeGroup = state.activeMessages.length
      ? {
          id: buildTaskGroupId(state.activeMessages),
          isCompleted: false,
          messages: state.activeMessages
        }
      : null

    const completedLimit = activeGroup
      ? visibleCompletedTaskGroupCount.value
      : visibleCompletedTaskGroupCount.value + 1
    const visibleCompletedGroups = state.completedGroups.slice(-completedLimit)

    return {
      groups: activeGroup ? [...visibleCompletedGroups, activeGroup] : visibleCompletedGroups,
      activeGroupId: activeGroup?.id || ''
    }
  })

  const hiddenCompletedTaskGroupCount = computed(() => {
    const completedLimit = taskWindowState.value.activeMessages.length
      ? visibleCompletedTaskGroupCount.value
      : visibleCompletedTaskGroupCount.value + 1
    return Math.max(0, taskWindowState.value.completedGroups.length - completedLimit)
  })

  const enhanceRawMessages = rawMsgs => {
    if (!rawMsgs.length) return []

    const toolStates = new Map() // tool_call_id -> { isFinal: bool, isRejected: bool, hasError: bool, isRunning: bool }
    const toolHasWaitingMsg = new Set() // tool_call_id that has an 'Awaiting' message
    const toolMessageIds = new Set() // tool_call_id with dedicated tool/user-observe messages
    const subAgentCompletions = new Map()
    const rejectedUserMessageIds = new Set()
    const ledgerStateById = new Map((workflowStore.toolList || []).map(tool => [tool.toolCallId, tool]))
    const subAgentProgressById = workflowStore.subAgentProgress || new Map()

    const tryParseJsonValue = value => {
      if (value === null || value === undefined) return null
      if (typeof value === 'object') return value
      if (typeof value !== 'string') return null

      const trimmed = value.trim()
      if (!trimmed) return null
      if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return null

      try {
        return JSON.parse(trimmed)
      } catch {
        return null
      }
    }

    const toBulletList = items =>
      items
        .map(item => {
          if (!item || typeof item !== 'object') {
            return `- ${String(item ?? '').trim()}`
          }

          const severity = String(item.severity || '').trim()
          const file = String(item.file || '').trim()
          const detail = String(item.detail || item.summary || '').trim()
          const labelParts = [severity && `**${severity}**`, file && `\`${file}\``].filter(Boolean)
          const label = labelParts.length ? `${labelParts.join(' ')}:` : '-'
          return `${label} ${detail || JSON.stringify(item)}`
        })
        .join('\n')

    const formatSubAgentResultMarkdown = value => {
      const parsed = tryParseJsonValue(value)
      if (!parsed) {
        return typeof value === 'string' ? value : String(value ?? '')
      }

      if (Array.isArray(parsed)) {
        return `\`\`\`json\n${JSON.stringify(parsed, null, 2)}\n\`\`\``
      }

      const approved = parsed.approved
      const summary = typeof parsed.summary === 'string' ? parsed.summary.trim() : ''
      const findings = Array.isArray(parsed.findings) ? parsed.findings : []
      const requiredFixes = Array.isArray(parsed.required_fixes)
        ? parsed.required_fixes
        : Array.isArray(parsed.requiredFixes)
          ? parsed.requiredFixes
          : []

      if (
        Object.prototype.hasOwnProperty.call(parsed, 'approved') ||
        summary ||
        findings.length > 0 ||
        requiredFixes.length > 0
      ) {
        const sections = []

        if (typeof approved === 'boolean') {
          sections.push(`**Verdict:** ${approved ? 'Approved' : 'Changes Required'}`)
        }

        if (summary) {
          sections.push(`**Summary**\n\n${summary}`)
        }

        if (findings.length > 0) {
          sections.push(`**Findings**\n\n${toBulletList(findings)}`)
        }

        if (requiredFixes.length > 0) {
          sections.push(`**Required Fixes**\n\n${toBulletList(requiredFixes)}`)
        }

        return sections.join('\n\n')
      }

      return `\`\`\`json\n${JSON.stringify(parsed, null, 2)}\n\`\`\``
    }

    const buildExplorationBatch = message => {
      if (message?.metadata?.message_kind !== 'exploration_batch') return null

      const parsed = tryParseJsonValue(message?.message)
      if (!parsed || typeof parsed !== 'object') return null

      const groups = Array.isArray(parsed.groups)
        ? parsed.groups.map(group => {
            const thought = typeof group?.thought === 'string' ? group.thought.trim() : ''
            const tools = Array.isArray(group?.tools)
              ? group.tools.map((tool, toolIndex) => {
                  const name = tool?.tool_name || tool?.toolName || tool?.name || ''
                  const args =
                    tool?.args && typeof tool.args === 'object'
                      ? tool.args
                      : tool?.arguments && typeof tool.arguments === 'object'
                        ? tool.arguments
                        : {}
                  const { icon, toolType, action, target } = formatToolTitle(name, args)
                  const messageContent =
                    typeof tool?.message === 'string'
                      ? tool.message
                      : typeof tool?.content === 'string'
                        ? tool.content
                        : ''
                  return {
                    id: `${message.id || message.stepIndex || 'exploration'}:${toolIndex}`,
                    icon,
                    toolType,
                    action,
                    target,
                    summary: typeof tool?.summary === 'string' ? tool.summary.trim() : '',
                    displayType: typeof tool?.display_type === 'string' ? tool.display_type : 'text',
                    message: messageContent,
                    sourceMessage: {
                      ...message,
                      message: messageContent,
                      metadata: {
                        ...(message.metadata || {}),
                        display_type: tool?.display_type || 'text'
                      }
                    }
                  }
                })
              : []

            return {
              thought,
              tools
            }
          })
        : []

      const files = groups.flatMap(group =>
        group.tools
          .map(tool => tool.target || '')
          .filter(target => typeof target === 'string' && target.trim())
      )

      return {
        groups,
        files,
        readCount: groups.reduce(
          (count, group) => count + group.tools.filter(tool => tool.action.startsWith('Read ')).length,
          0
        ),
        searchCount: groups.reduce(
          (count, group) =>
            count +
            group.tools.filter(
              tool =>
                tool.action.startsWith('Search ') ||
                tool.action.startsWith('Grep ') ||
                tool.action.startsWith('Glob ')
            ).length,
          0
        ),
        thoughtCount: groups.filter(group => group.thought).length
      }
    }

    const parseSubAgentRunPayload = message => {
      const meta = message?.metadata || {}
      const observationData = meta.data || {}
      const taskId =
        meta.sub_agent_id || meta.subAgentId || observationData.sub_agent_id || observationData.subAgentId || ''
      const mode = meta.sub_agent_mode || meta.subAgentMode || observationData.sub_agent_mode || observationData.subAgentMode || ''
      const task =
        meta.sub_agent_task ||
        meta.subAgentTask ||
        observationData.sub_agent_task ||
        observationData.subAgentTask ||
        ''
      return {
        taskId,
        mode,
        task,
        agent:
          meta.sub_agent_name ||
          meta.subAgentName ||
          observationData.sub_agent_name ||
          observationData.subAgentName ||
          ''
      }
    }

    const buildSubAgentCard = message => {
      const meta = message?.metadata || {}
      const observationData = meta.data || {}
      const toolName = String(meta.tool_name || '').toLowerCase()
      const directTaskId =
        meta.sub_agent_id || meta.subAgentId || meta.data?.sub_agent_id || meta.data?.subAgentId || ''
      if (toolName !== 'sub_agent_run' && !directTaskId) return null

      const payload =
        toolName === 'sub_agent_run'
          ? parseSubAgentRunPayload(message)
          : {
              taskId: directTaskId,
              mode: meta.sub_agent_mode || meta.subAgentMode || 'call',
              task:
                meta.sub_agent_task ||
                meta.subAgentTask ||
                meta.data?.sub_agent_task ||
                meta.data?.subAgentTask ||
                '',
              agent:
                meta.sub_agent_name ||
                meta.subAgentName ||
                meta.data?.sub_agent_name ||
                meta.data?.subAgentName ||
                ''
            }
      const childWorkflow = payload.taskId
        ? workflowStore.workflows?.find?.(workflow => workflow?.id === payload.taskId) || null
        : null
      const childExecutionContext = childWorkflow?.executionContext || {}
      const summary = payload.taskId ? childAgentSummaryById.value.get(payload.taskId) : null
      const liveProgress = payload.taskId ? subAgentProgressById.get(payload.taskId) : null
      const aggregateProgress = {
        ...(liveProgress || {}),
        agentName: pickPreferredText(
          liveProgress?.agentName,
          liveProgress?.agent_name,
          summary?.agentName
        ),
        task: pickPreferredText(liveProgress?.task, summary?.task),
        toolCallsCount: pickPreferredNumber(
          liveProgress?.toolCallsCount,
          liveProgress?.tool_calls_count,
          summary?.toolCalls
        )
      }
      const completion = payload.taskId ? subAgentCompletions.get(payload.taskId) : null
      const completionData = completion?.data || {}
      const completionResult = completion?.result || completionData.result || {}
      const completionStatus =
        completion?.execution_status ||
        completionData.execution_status ||
        completionResult.status ||
        liveProgress?.status ||
        meta.sub_agent_status ||
        observationData.execution_status ||
        meta.execution_status ||
        'running'
      const toolCallsCount =
        completionResult.tool_calls_count ??
        completionResult.toolCallsCount ??
        completion?.tool_calls_count ??
        completionData.tool_calls_count ??
        completionData.toolCallsCount ??
        liveProgress?.toolCallsCount ??
        liveProgress?.tool_calls_count ??
        0
      const currentContextTokens =
        completionResult.current_context_tokens ??
        completionResult.currentContextTokens ??
        completion?.current_context_tokens ??
        completionData.current_context_tokens ??
        completionData.currentContextTokens ??
        childExecutionContext.currentContextTokens ??
        childExecutionContext.current_context_tokens ??
        null
      const maxContextTokens =
        completionResult.max_context_tokens ??
        completionResult.maxContextTokens ??
        completion?.max_context_tokens ??
        completionData.max_context_tokens ??
        completionData.maxContextTokens ??
        childExecutionContext.maxContextTokens ??
        childExecutionContext.max_context_tokens ??
        null
      const rawResultValue =
        completionResult.result ??
        completionResult.error ??
        completionData.result ??
        completionData.error ??
        completion?.result ??
        completion?.summary ??
        completionData.summary ??
        ''
      const resultContent =
        typeof rawResultValue === 'string'
          ? rawResultValue
          : JSON.stringify(rawResultValue, null, 2)
      const resultMarkdown = formatSubAgentResultMarkdown(rawResultValue)
      const hasResult =
        typeof rawResultValue === 'string'
          ? rawResultValue.trim().length > 0
          : Array.isArray(rawResultValue)
            ? rawResultValue.length > 0
            : !!(rawResultValue && typeof rawResultValue === 'object'
              ? Object.keys(rawResultValue).length
              : rawResultValue)

      return {
        taskId: payload.taskId,
        agent:
          payload.agent ||
          completion?.sub_agent_name ||
          completionData.sub_agent_name ||
          completionData.subAgentName ||
          observationData.sub_agent_name ||
          observationData.subAgentName ||
          aggregateProgress?.agentName ||
          'Sub-agent',
        task:
          payload.task ||
          completion?.sub_agent_task ||
          completionData.sub_agent_task ||
          completionData.subAgentTask ||
          observationData.sub_agent_task ||
          observationData.subAgentTask ||
          aggregateProgress?.task ||
          'Delegated task',
        taskMarkdown:
          payload.task ||
          completion?.sub_agent_task ||
          completionData.sub_agent_task ||
          completionData.subAgentTask ||
          observationData.sub_agent_task ||
          observationData.subAgentTask ||
          aggregateProgress?.task ||
          'Delegated task',
        mode: payload.mode || 'call',
        status: summary?.status === 'success' ? 'completed' : summary?.status || completionStatus,
        toolCallsCount,
        currentContextTokens,
        maxContextTokens,
        contextPercent:
          typeof summary?.contextPercent === 'number' ? summary.contextPercent : null,
        result: resultContent,
        resultMarkdown,
        hasResult
      }
    }

    // --- PASS 1: Single scan to collect all states (O(N)) ---
    const processedMsgs = rawMsgs.map(m => {
      let meta = m.metadata
      // Note: metadata is already an object (serde_json::Value from Rust)
      // No need to parse, but we should handle null/undefined
      if (!meta) {
        meta = {}
      }

      // Check for tool messages OR rejected user messages with tool_call_id
      const hasToolCallId = meta?.tool_call_id
      if (hasToolCallId) {
        const id = meta.tool_call_id
        const approvalStatus = meta.approval_status || ''
        const executionStatus = meta.execution_status || ''
        const ledgerState = ledgerStateById.get(id)

        if (m.role === 'tool' || (m.role === 'user' && m.stepType === 'observe')) {
          toolMessageIds.add(id)
        }

        if (m.role === 'user' && approvalStatus === 'rejected') {
          rejectedUserMessageIds.add(id)
        }

        if (ledgerState?.status === 'approved_running') {
          toolStates.set(id, {
            isFinal: false,
            isRejected: false,
            hasError: false,
            isRunning: true
          })
        } else if (ledgerState?.status === 'rejected') {
          toolStates.set(id, { isFinal: true, isRejected: true, hasError: false })
        } else if (ledgerState?.status === 'final_success') {
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: false })
        } else if (ledgerState?.status === 'final_error') {
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: true })
        } else if (executionStatus === 'pending_approval' || approvalStatus === 'pending') {
          toolHasWaitingMsg.add(id)
        } else if (executionStatus === 'approval_submitted' || executionStatus === 'running') {
          toolStates.set(id, {
            isFinal: false,
            isRejected: false,
            hasError: false,
            isRunning: true
          })
        } else if (
          executionStatus === 'completed' ||
          executionStatus === 'failed' ||
          executionStatus === 'rejected'
        ) {
          const isRejected = executionStatus === 'rejected'
          const isError =
            executionStatus === 'failed' || m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected, hasError: isError })
        } else if (approvalStatus === 'rejected') {
          // Final states
          const isError = m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected: true, hasError: isError })
        } else if (approvalStatus === 'approved' && executionStatus !== 'running') {
          const isError = m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: isError })
        } else if (m.role === 'tool') {
          // Fallback: normal tool execution result (no approval flow)
          const isError = m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: isError })
        }
      }

      const completionId =
        meta?.sub_agent_id || meta?.subAgentId || meta?.data?.sub_agent_id || meta?.data?.subAgentId
      if (meta?.observation_type === 'sub_agent_completion' && completionId) {
        subAgentCompletions.set(completionId, {
          summary: meta.summary || '',
          execution_status: meta.execution_status || '',
          result: meta.result || {},
          sub_agent_name: meta.sub_agent_name || meta.subAgentName || '',
          sub_agent_task: meta.sub_agent_task || meta.subAgentTask || '',
          data: meta.data || {}
        })
      }

      return { ...m, metadata: meta } // Cache parsed meta for Pass 2
    })

    // --- PASS 2: Filter and Transform (O(N)) ---
    return processedMsgs
      .filter(m => {
        if (m.metadata?.ui_visibility === 'hide' || m.metadata?.uiVisibility === 'hide') {
          return false
        }

        // Hide redundancy for tool-related messages
        if (m.metadata?.tool_call_id) {
          const id = m.metadata.tool_call_id
          const state = toolStates.get(id)
          const approvalStatus = m.metadata.approval_status
          const ledgerState = ledgerStateById.get(id)
          const isResolvedByLedger =
            ledgerState?.status === 'approved_running' ||
            ledgerState?.status === 'rejected' ||
            ledgerState?.status === 'final_success' ||
            ledgerState?.status === 'final_error'

          // If there's a final result (approved, rejected, or executed)
          if (state?.isFinal || state?.isRunning || isResolvedByLedger) {
            // Hide "pending" messages when there's a final result
            if (approvalStatus === 'pending' && toolHasWaitingMsg.has(id)) return false
          }
        }

        // Hide user messages with stepType 'observe' (internal system messages)
        // BUT keep rejected messages which have tool_call_id
        if (m.role === 'user' && m.stepType === 'observe' && !m.metadata?.tool_call_id) {
          if (m.metadata?.ui_visibility === 'show' || m.metadata?.ui_visibility === 'card') {
            return true
          }
          if (m.metadata?.ui_visibility === 'hide') return false
          return false
        }

        return true
      })
      .flatMap((message, idx) => {
        const toolDisplay = getToolDisplayInfo(message)
        const displayId = message.id || `msg_${message.role}_${message.stepIndex}_${idx}`

        let isRejected = false
        let isApproved = false

        // Check approval status from metadata (preferred method)
        const approvalStatus = message.metadata?.approval_status
        const executionStatus = message.metadata?.execution_status
        if (approvalStatus === 'rejected' || executionStatus === 'rejected') {
          isRejected = true
        } else if (executionStatus === 'running') {
          isApproved = false
        } else if (approvalStatus === 'approved') {
          isApproved = true
        } else if (message.metadata?.tool_call_id) {
          // Fallback: Check tool states for backward compatibility
          const state = toolStates.get(message.metadata.tool_call_id)
          if (state?.isFinal) {
            if (state.isRejected) isRejected = true
            else isApproved = true
          }
        }

        // Pre-calculate pending tool calls
        let pendingToolCalls = []
        const toolCalls = message.metadata?.tool_calls || []
        if (Array.isArray(toolCalls) && toolCalls.length > 0) {
          pendingToolCalls = toolCalls
            .map(call => {
              const name = call.function?.name || call.name || ''
              const rawArgs = call.function?.arguments || call.arguments || {}
              let args = rawArgs
              if (typeof rawArgs === 'string') {
                try {
                  args = JSON.parse(rawArgs)
                } catch (e) {
                  args = {}
                }
              }
              const { icon, toolType, action, target } = formatToolTitle(name, args)
              const state = toolStates.get(call.id)
              const ledgerState = workflowStore.toolList?.find(tool => tool.toolCallId === call.id)
              const isRejected =
                ledgerState?.status === 'rejected' || (!!state?.isFinal && !!state?.isRejected)
              const isRunning = ledgerState?.status === 'approved_running' || !!state?.isRunning
              const completionSummary =
                name === 'complete_workflow_with_summary' && typeof args.summary === 'string'
                  ? args.summary.trim()
                  : ''
              return {
                id: call.id,
                icon,
                toolType,
                action,
                target,
                isRejected,
                toolName: name,
                completionSummary,
                summary: getToolStatusSummary(
                  name,
                  isRejected
                    ? 'rejected'
                    : isRunning || isAutoExecuteWorkflowTool(name)
                      ? 'running'
                      : 'pending',
                  isRejected
                    ? 'User rejected'
                    : isRunning || isAutoExecuteWorkflowTool(name)
                      ? 'Executing...'
                      : 'Awaiting approval'
                )
              }
            })
            .filter(call => {
              if (isInternalTodoTool(call.toolName)) return false
              if (call.toolName === 'sub_agent_run') return false
              if (toolMessageIds.has(call.id)) return false
              const state = toolStates.get(call.id)
              if (!state) return true
              return state.isRejected
            })
        }

        const enhancedMessage = {
          ...message,
          displayId,
          toolDisplay,
          explorationBatch: buildExplorationBatch(message),
          subAgentCard: buildSubAgentCard(message),
          pendingToolCalls,
          isRejected,
          isApproved
        }

        const syntheticMessages = [enhancedMessage]
        const rejectionMessage = String(message.metadata?.rejection_message || '').trim()
        const toolCallId = String(message.metadata?.tool_call_id || '').trim()

        if (
          message.role === 'tool' &&
          isRejected &&
          rejectionMessage &&
          toolCallId &&
          !rejectedUserMessageIds.has(toolCallId)
        ) {
          rejectedUserMessageIds.add(toolCallId)
          syntheticMessages.push({
            id: `${displayId}_rejection_user`,
            displayId: `${displayId}_rejection_user`,
            role: 'user',
            stepType: 'Observe',
            stepIndex: `${message.stepIndex || idx}_rejection`,
            message: rejectionMessage,
            metadata: {
              tool_call_id: toolCallId,
              approval_status: 'rejected',
              ui_visibility: 'show'
            }
          })
        }

        return syntheticMessages
      })
      .filter(m => {
        if (m.metadata?.ui_visibility === 'hide') return false
        if (m.role === 'tool') {
          const name =
            m.metadata?.tool_name ||
            m.metadata?.tool_call?.name ||
            m.metadata?.tool_call?.function?.name ||
            ''
          if (name === 'answer_user') return false
          if (
            m.metadata?.execution_status === 'running' &&
            !workflowStore.getToolStream(m.metadata?.tool_call_id).length
          ) {
            return true
          }
          return true
        }
        if (m.role === 'user' && m.metadata?.approval_status === 'rejected') {
          const visibleContent = removeSystemReminder(m.message || '')
          return !!visibleContent
        }
        if (m.role === 'assistant') {
          const hasTextContent =
            (m.message && m.message.trim()) || (m.reasoning && m.reasoning.trim())
          if (hasTextContent) return true
          if (m.pendingToolCalls && m.pendingToolCalls.length > 0) return true
          return false
        }
        return true
      })
  }

  const rawEnhancedMessages = computed(() => {
    void childAgentSummaryById.value
    void childAgentSummariesRevision.value
    const { groups, activeGroupId } = visibleTaskGroupsState.value
    if (!groups.length) return []

    const visibleGroupIds = new Set(groups.map(group => group.id))
    for (const cachedGroupId of taskGroupCache.keys()) {
      if (!visibleGroupIds.has(cachedGroupId)) {
        taskGroupCache.delete(cachedGroupId)
      }
    }

    return groups.flatMap(group => {
      const signature = buildTaskGroupSignature(group.messages)
      const cachedEntry = taskGroupCache.get(group.id)

      if (group.isCompleted && group.id !== activeGroupId && cachedEntry?.signature === signature) {
        const summarySignature = childAgentSummariesRevision.value
        if (cachedEntry.summarySignature === summarySignature) {
          return cachedEntry.messages
        }
      }

      const enhanced = reuseUnchangedEnhancedMessages(cachedEntry, enhanceRawMessages(group.messages))
      taskGroupCache.set(group.id, {
        signature,
        summarySignature: childAgentSummariesRevision.value,
        messages: enhanced.messages,
        messageSignatures: enhanced.messageSignatures
      })
      return enhanced.messages
    })
  })

  const enhancedMessages = computed(() => rawEnhancedMessages.value)

  const lastAssistantMessage = computed(() => {
    for (let index = enhancedMessages.value.length - 1; index >= 0; index -= 1) {
      const message = enhancedMessages.value[index]
      if (message?.role === 'assistant') return message
    }
    return null
  })

  const toggleMessageExpand = id => {
    if (expandedMessages.value.has(id)) {
      expandedMessages.value.delete(id)
    } else {
      expandedMessages.value.add(id)
    }
  }

  const isMessageExpanded = message => {
    // Only force expansion for 'Ask User' to ensure visibility of interaction points.
    // Everything else (especially heavy Diffs) should be collapsed by default.
    if (message.metadata?.approval_status === 'pending') return true
    if (message.toolDisplay?.action === 'Ask User') return true
    return expandedMessages.value.has(message.displayId)
  }

  const toggleReasoningExpand = id => {
    if (expandedReasonings.value.has(id)) {
      expandedReasonings.value.delete(id)
    } else {
      expandedReasonings.value.add(id)
    }
  }

  const isReasoningExpanded = id => expandedReasonings.value.has(id)

  // Helper functions for truncating text (UTF-8 safe)
  const truncateText = (text, maxLen = 25) => {
    if (!text) return ''
    const chars = Array.from(text)
    if (chars.length <= maxLen) return text
    return chars.slice(0, maxLen - 3).join('') + '...'
  }

  const truncatePath = (path, maxLen = 30) => {
    if (!path || path.length <= maxLen) return path
    // For paths, try to keep the filename and truncate the middle
    const parts = path.split('/')
    const fileName = parts.pop()
    if (fileName && fileName.length > maxLen - 10) {
      return '.../' + truncateText(fileName, maxLen - 4)
    }
    const dir = parts.join('/')
    const available = maxLen - fileName.length - 4 // 4 for ".../"
    if (available > 5) {
      return truncateText(dir, available) + '/.../' + fileName
    }
    return '.../' + fileName
  }

  const displayRoots = () => {
    const workflow = workflowStore.currentWorkflow
    const roots = [
      ...(Array.isArray(workflow?.allowedPaths) ? workflow.allowedPaths : []),
      ...(Array.isArray(workflow?.agentConfig?.allowedPaths) ? workflow.agentConfig.allowedPaths : [])
    ]
    return [...new Set(roots.filter(Boolean))]
  }

  const isInternalTodoTool = toolName => String(toolName || '').toLowerCase().startsWith('todo_')

  const decodeCompatJsonPayload = value => {
    if (typeof value !== 'string') return value
    const trimmed = value.trim()
    if (!trimmed) return value
    const looksLikeJson =
      trimmed.startsWith('{') ||
      trimmed.startsWith('[') ||
      (trimmed.startsWith('"') && (trimmed.includes('{') || trimmed.includes('[')))
    if (!looksLikeJson) return value

    let current = value
    for (let depth = 0; depth < 2; depth += 1) {
      if (typeof current !== 'string') break
      try {
        current = JSON.parse(current)
      } catch {
        break
      }
    }
    return current
  }

  // Format tool title with icon, tool type class, and display text
  const formatToolTitle = (name, args) => {
    const toolFormatters = {
      read_file: args => {
        const path = formatDisplayPath(args.file_path || args.path || '', displayRoots())
        const limit = args.limit
        const offset = args.offset
        let suffix = ''
        if (limit !== undefined && offset !== undefined) {
          suffix = ` L${offset + 1}-${offset + limit}`
        } else if (limit !== undefined) {
          suffix = ` L1-${limit}`
        } else if (offset !== undefined) {
          suffix = ` L${offset + 1}`
        }
        return {
          icon: resolveWorkflowToolIcon(name, 'file'),
          toolType: 'tool-file',
          action: 'Read',
          target: `${path}${suffix}`
        }
      },

      write_file: args => {
        const path = formatDisplayPath(args.file_path || args.path || '', displayRoots())
        return {
          icon: resolveWorkflowToolIcon(name, 'file'),
          toolType: 'tool-file',
          action: 'Write',
          target: path
        }
      },

      edit_file: args => {
        const path = formatDisplayPath(args.file_path || args.path || '', displayRoots())
        return {
          icon: resolveWorkflowToolIcon(name, 'edit'),
          toolType: 'tool-file',
          action: 'Edit',
          target: path
        }
      },

      list_dir: args => {
        const path = formatDisplayPath(args.path || args.dir || '.', displayRoots())
        return {
          icon: resolveWorkflowToolIcon(name, 'folder'),
          toolType: 'tool-file',
          action: 'List',
          target: path
        }
      },

      glob: args => {
        const pattern = args.pattern || args.glob || ''
        const path = formatDisplayPath(args.path || '', displayRoots())
        return {
          icon: resolveWorkflowToolIcon(name, 'search'),
          toolType: 'tool-file',
          action: `Glob ${pattern}`,
          target: path
        }
      },

      grep: args => {
        const pattern = args.pattern || args.query || ''
        const path = formatDisplayPath(args.path || '', displayRoots())
        const action = path ? `Grep "${pattern}" in ${path}` : `Grep "${pattern}"`
        return {
          icon: resolveWorkflowToolIcon(name, 'search'),
          toolType: 'tool-file',
          action,
          target: ''
        }
      },

      web_fetch: args => {
        const url = args.url || ''
        return {
          icon: resolveWorkflowToolIcon(name, 'link'),
          toolType: 'tool-network',
          action: `Fetch ${url}`,
          target: ''
        }
      },

      web_search: args => {
        const query = args.query || ''
        const numResults = args.num_results
        const action =
          numResults !== undefined
            ? `Search "${query}" (Count: ${numResults})`
            : `Search "${query}"`
        return {
          icon: resolveWorkflowToolIcon(name, 'search'),
          toolType: 'tool-network',
          action,
          target: ''
        }
      },

      bash: args => {
        const cmd = normalizeShellCommandForDisplay(args.command || '', displayRoots())
        return {
          icon: resolveWorkflowToolIcon(name, 'terminal'),
          toolType: 'tool-system',
          action: `Bash: ${cmd}`,
          target: ''
        }
      },
      sub_agent_run: args => {
        const childAgent = args.child_agent_name || args.child_agent_id || ''
        const mode = args.execution_mode || 'call'
        return {
          icon: resolveWorkflowToolIcon(name, 'task'),
          toolType: 'tool-system',
          action: mode === 'background' ? 'Run Sub-agent in Background' : 'Run Sub-agent',
          target: childAgent
        }
      },
      sub_agent_output: args => ({
        icon: resolveWorkflowToolIcon(name, 'task'),
        toolType: 'tool-system',
        action: 'Get Sub-agent Output',
        target: args.task_id || ''
      }),
      sub_agent_stop: args => ({
        icon: resolveWorkflowToolIcon(name, 'stop'),
        toolType: 'tool-system',
        action: 'Stop Sub-agent',
        target: args.task_id || ''
      }),

      todo_create: args => {
        // Handle single todo creation
        const subject = args.subject || args.title || ''
        if (subject) {
          return {
            icon: resolveWorkflowToolIcon(name, 'add'),
            toolType: 'tool-todo',
            action: t('workflow.todo.create'),
            target: truncateText(subject, 25)
          }
        }
        // Handle batch creation
        const tasks = args.tasks
        if (tasks && Array.isArray(tasks)) {
          return {
            icon: resolveWorkflowToolIcon(name, 'add'),
            toolType: 'tool-todo',
            action: t('workflow.todo.createBatch'),
            target: `${tasks.length}项`
          }
        }
        return {
          icon: resolveWorkflowToolIcon(name, 'add'),
          toolType: 'tool-todo',
          action: t('workflow.todo.create'),
          target: ''
        }
      },

      todo_update: args => {
        const subject = args.subject || args.title || ''
        const status = args.status || ''
        let statusText = ''
        if (status === 'completed') statusText = t('workflow.todo.statusCompleted')
        else if (status === 'in_progress') statusText = t('workflow.todo.statusInProgress')
        else if (status === 'pending') statusText = t('workflow.todo.statusPending')
        else statusText = status

        if (subject && statusText) {
          return {
            icon: resolveWorkflowToolIcon(name, 'check'),
            toolType: 'tool-todo',
            action: `Update ${truncateText(subject, 20)} to ${statusText}`,
            target: ''
          }
        }
        return {
          icon: resolveWorkflowToolIcon(name, 'check'),
          toolType: 'tool-todo',
          action: t('workflow.todo.update'),
          target: ''
        }
      },
      todo_list: () => ({
        icon: resolveWorkflowToolIcon(name, 'list'),
        toolType: 'tool-todo',
        action: t('workflow.todo.list'),
        target: ''
      }),
      todo_get: () => ({
        icon: resolveWorkflowToolIcon(name, 'list'),
        toolType: 'tool-todo',
        action: t('workflow.todo.view'),
        target: ''
      }),
      complete_workflow_with_summary: () => ({
        icon: resolveWorkflowToolIcon(name, 'check-circle'),
        toolType: 'tool-todo',
        action: t('workflow.finishTask'),
        target: ''
      })
    }

    const formatter = toolFormatters[name]
    if (formatter) {
      return formatter(args || {})
    }

    // Default handling - just show the tool name
    const defaultName = name.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
    return {
      icon: resolveWorkflowToolIcon(name, 'tool'),
      toolType: 'tool-system',
      action: defaultName,
      target: ''
    }
  }

  // Standardize tool display info from metadata
  const getToolDisplayInfo = message => {
    const meta = message.metadata || {}
    const isError = message.isError || message.is_error || meta.is_error || false

    // 1. Try to extract tool call info
    const toolCall = meta.tool_call || {}
    const func = toolCall.function || toolCall
    const toolCallId = meta.tool_call_id || toolCall.id || func.id
    const executionStatus = meta.execution_status || ''
    const hasStreamOutput = toolCallId ? workflowStore.getToolStream(toolCallId).length > 0 : false
    const name = func.name || toolCall.name || meta.tool_name || ''
    const rawArgs = func.arguments || func.input || {}

    let args = rawArgs
    if (typeof rawArgs === 'string') {
      try {
        args = JSON.parse(rawArgs)
      } catch (e) {
        args = {}
      }
    }
    if (!args || typeof args !== 'object' || Array.isArray(args)) {
      args = {}
    }

    const ledgerState = toolCallId
      ? (workflowStore.toolList || []).find(tool => tool.toolCallId === toolCallId)
      : null
    if ((!args || Object.keys(args).length === 0) && ledgerState?.arguments) {
      args = ledgerState.arguments
    }

    let parsedPayload = meta.details && typeof meta.details === 'object' ? meta.details : null

    if ((!args || Object.keys(args).length === 0) && parsedPayload && typeof parsedPayload === 'object') {
      args = parsedPayload
    }

    const canUseLegacyMessagePayload =
      typeof message.message === 'string' &&
      !parsedPayload &&
      ['diff', 'markdown', 'text'].includes(meta.display_type || '') &&
      !!toolCallId

    if ((!args || Object.keys(args).length === 0) && canUseLegacyMessagePayload) {
      const parsedDetails = decodeCompatJsonPayload(message.message)
      if (parsedDetails && typeof parsedDetails === 'object') {
        args = parsedDetails
        parsedPayload = parsedDetails
      }
    }

    if (!parsedPayload && canUseLegacyMessagePayload) {
      const parsedDetails = decodeCompatJsonPayload(message.message)
      if (parsedDetails && typeof parsedDetails === 'object') {
        parsedPayload = parsedDetails
      }
    }

    // 2. Format using standard rules
    const formatted = formatToolTitle(name, args)

    // 3. Robust Priority:
    // If backend provided a title explicitly, use it as the main action.
    // This is crucial for results (observations) where original tool_call might be obscured.
    let finalAction = formatted.action
    let finalTarget = formatted.target
    let finalIcon = formatted.icon
    let finalToolType = formatted.toolType

    if (name === 'complete_workflow_with_summary') {
      finalAction = t('workflow.finishTask')
      finalTarget = ''
    } else if (typeof meta.title === 'string' && meta.title.trim()) {
      finalAction = normalizeToolDisplayText(removeSystemReminder(meta.title), displayRoots())
      const normalizedFormattedAction = normalizeToolDisplayText(formatted.action || '', displayRoots())
      const normalizedFinalAction = normalizeToolDisplayText(finalAction || '', displayRoots())
      const titleAlreadyIncludesTarget = finalTarget && normalizedFinalAction.includes(finalTarget)
      if (
        !finalTarget ||
        titleAlreadyIncludesTarget ||
        normalizedFinalAction !== normalizedFormattedAction
      ) {
        finalTarget = ''
      }
    }

    // Fallback for missing action (prevents empty titles)
    if (!finalAction && !name) {
      // If it's a tool result but we lost the name, use a generic "Result"
      finalAction = t('chat.toolResult') || 'Result'
    }

    const fallbackSummary = removeSystemReminder(meta.summary || (isError ? 'Failed' : 'Executing...'))
    const summaryStatus =
      executionStatus === 'pending_approval'
        ? 'pending'
        : executionStatus === 'running'
          ? 'running'
          : executionStatus === 'rejected'
            ? 'rejected'
            : executionStatus === 'completed'
              ? isError
                ? 'failed'
                : 'success'
              : executionStatus === 'failed'
                ? 'failed'
                : meta.approval_status === 'pending'
                  ? 'pending'
                  : meta.approval_status === 'approved'
                    ? 'running'
                    : meta.approval_status === 'rejected'
                      ? 'rejected'
                      : isError
                        ? 'failed'
                        : undefined

    const looksLikeFileChangePayload = payload => {
      if (!payload || typeof payload !== 'object') return false
      const hasPath =
        typeof payload.file_path === 'string' ||
        typeof payload.path === 'string' ||
        typeof payload.display_path === 'string'
      const hasEditFields =
        payload.old_string !== undefined ||
        payload.new_string !== undefined ||
        payload.content !== undefined
      return hasPath && hasEditFields
    }

    const inferredDisplayType =
      meta.display_type ||
      (['edit_file', 'write_file', 'plan_edit_note', 'plan_write_note'].includes(name)
        ? 'diff'
        : looksLikeFileChangePayload(parsedPayload) || looksLikeFileChangePayload(args)
          ? 'diff'
          : 'text')

    return {
      title: finalAction + (finalTarget ? ` ${finalTarget}` : ''),
      summary:
        name === 'complete_workflow_with_summary'
          ? ''
          : getToolStatusSummary(name, summaryStatus, fallbackSummary),
      isError: isError,
      displayType: inferredDisplayType,
      icon: finalIcon,
      toolType: finalToolType,
      action: finalAction,
      target: finalTarget,
      hasStreamOutput,
      executionStatus
    }
  }

  const shouldShowToolRawContent = message => {
    const meta = message.metadata || {}
    const content = removeSystemReminder(message.message || '')
    if (!content) return false
    if (meta.hide_approval_details && meta.execution_status === 'running') return false
    if (
      (message.toolDisplay?.hasStreamOutput ||
        workflowStore.getToolStream(meta.tool_call_id).length > 0) &&
      message.toolDisplay?.displayType === 'text'
    ) {
      return false
    }
    return true
  }

  const appendContextDiffMarkdown = (parts, lines, startLine) => {
    if (!Array.isArray(lines) || !lines.length) return
    lines.forEach((line, index) => {
      const lineNum = (startLine + index).toString().padStart(4, ' ')
      parts.push(`  ${lineNum} | ${line}`)
    })
  }

  // Get diff markdown for file edits
  const getDiffMarkdown = content => {
    try {
      let data = content
      if (typeof content === 'string') {
        try {
          data = JSON.parse(content)
        } catch (e) {
          return content
        }
      }

      const oldStr = data.old_string !== undefined ? data.old_string : ''
      const newStr = data.new_string !== undefined ? data.new_string : data.content || ''
      const startLine = data.start_line || 1

      // If it's just raw content without diff semantics, return as code block
      if (data.old_string === undefined && data.new_string === undefined && !data.content) {
        return typeof content === 'string' ? content : JSON.stringify(content, null, 2)
      }

      // const filePath = data.file_path || data.path || 'file'
      // const oldLinesCount = oldStr.split('\n').length
      // const newLinesCount = newStr.split('\n').length
      // Generate standard unidiff-like format with line numbers
      // let diffContent = `File: **${filePath}**\n`
      // if (data.start_line) {
      //     diffContent += `Range: L${startLine} - L${startLine + Math.max(oldLinesCount, newLinesCount) - 1}\n`
      // }
      const diffParts = ['```diff']
      appendContextDiffMarkdown(
        diffParts,
        data.context_before,
        data.context_before_start_line || Math.max(1, startLine - (data.context_before?.length || 0))
      )

      const UI_LINE_LIMIT = 3000 // Limit lines shown in UI for performance

      if (data.old_string !== undefined) {
        // Use diff library to generate proper line-by-line diff
        const changes = Diff.diffLines(oldStr, newStr)
        let lineCount = 0
        let currentLineOld = startLine
        let currentLineNew = startLine

        changes.forEach(change => {
          if (lineCount >= UI_LINE_LIMIT) return

          const lines = change.value.split('\n')
          // Remove last empty line if exists
          if (lines[lines.length - 1] === '') {
            lines.pop()
          }

          lines.forEach(line => {
            if (lineCount >= UI_LINE_LIMIT) return

            const lineNumDisplay = change.added ? currentLineNew : currentLineOld
            const lineNumStr = lineNumDisplay.toString().padStart(4, ' ')

            if (change.added) {
              diffParts.push(`+ ${lineNumStr} | ${line}`)
              currentLineNew++
              lineCount++
            } else if (change.removed) {
              diffParts.push(`- ${lineNumStr} | ${line}`)
              currentLineOld++
              lineCount++
            } else {
              diffParts.push(`  ${lineNumStr} | ${line}`)
              currentLineOld++
              currentLineNew++
              lineCount++
            }
          })
        })

        if (lineCount >= UI_LINE_LIMIT) {
          diffParts.push('... (truncated for preview)')
        }
        appendContextDiffMarkdown(
          diffParts,
          data.context_after,
          data.context_after_start_line || currentLineOld
        )
      } else {
        diffParts.push(`- ${startLine.toString().padStart(4, ' ')} | (empty)`)
        const newLines = newStr.split('\n')
        const displayLines = newLines.slice(0, UI_LINE_LIMIT)

        displayLines.forEach((line: string, index: number) =>
          diffParts.push(`+ ${(startLine + index).toString().padStart(4, ' ')} | ${line}`)
        )
        if (newLines.length > UI_LINE_LIMIT) {
          diffParts.push(`+ ... (${newLines.length - UI_LINE_LIMIT} lines truncated)`)
        }
        appendContextDiffMarkdown(
          diffParts,
          data.context_after,
          data.context_after_start_line || startLine + displayLines.length
        )
      }

      diffParts.push('```')
      return diffParts.join('\n')
    } catch (e) {
      return typeof content === 'string' ? content : JSON.stringify(content)
    }
  }

  const normalizeChoiceGroups = parsed => {
    if (Array.isArray(parsed)) {
      const groups = parsed
        .map(group => ({
          title: typeof group?.title === 'string' ? group.title.trim() : '',
          options: Array.isArray(group?.options)
            ? group.options
                .filter(option => typeof option === 'string')
                .map(option => option.trim())
                .filter(Boolean)
            : []
        }))
        .filter(group => group.title && group.options.length > 0)

      return { groups }
    }

    if (parsed && typeof parsed === 'object') {
      const question = typeof parsed.question === 'string' ? parsed.question.trim() : ''
      const options = Array.isArray(parsed.options)
        ? parsed.options
            .filter(option => typeof option === 'string')
            .map(option => option.trim())
            .filter(Boolean)
        : []

      if (question || options.length > 0) {
        return {
          groups: [
            {
              title: question || t('workflow.waitingForUser') || 'Waiting for user',
              options
            }
          ].filter(group => group.options.length > 0)
        }
      }
    }

    if (typeof parsed === 'string' && parsed.trim()) {
      return {
        groups: [
          {
            title: parsed.trim(),
            options: []
          }
        ]
      }
    }

    return { groups: [] }
  }

  // Parse choice content for Ask User tool
  const parseChoiceContent = content => {
    try {
      return normalizeChoiceGroups(JSON.parse(content))
    } catch (e) {
      return normalizeChoiceGroups(content)
    }
  }

  // Helper to parse message content
  const getParsedMessage = message => {
    let content = message.message || ''
    content = removeSystemReminder(content)
    let toolCalls = []
    const isError = message.isError || message.is_error || false

    try {
      const trimmed = content.trim()
      if (trimmed.startsWith('{')) {
        const parsed = JSON.parse(trimmed)
        let parsedContent = parsed.content || ''
        let parsedToolCalls =
          parsed.tool_calls || parsed.toolCall || (parsed.tool ? [parsed.tool] : [])

        // Filter out internal tools
        parsedToolCalls = parsedToolCalls.filter(call => {
          const name = call?.function?.name || call?.name
          return name !== 'answer_user'
        })

        // If assistant Think step, hide tool calls
        if (message.role === 'assistant' && message.stepType === 'Think') {
          parsedToolCalls = []
        }

        return {
          content: parsedContent,
          toolCalls: parsedToolCalls,
          isError
        }
      }
    } catch (e) {
      // Not JSON
    }

    return {
      content,
      toolCalls: [],
      isError
    }
  }

  return {
    expandedMessages,
    expandedReasonings,
    enhancedMessages,
    hiddenCompletedTaskGroupCount,
    lastAssistantMessage,
    toggleMessageExpand,
    isMessageExpanded,
    toggleReasoningExpand,
    isReasoningExpanded,
    removeSystemReminder,
    formatToolTitle,
    getToolDisplayInfo,
    getDiffMarkdown,
    parseChoiceContent,
    getParsedMessage,
    shouldShowToolRawContent
  }
}
