import { ref, computed, watch } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
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
import { WORKFLOW_STATUSES, WORKFLOW_WAIT_REASONS } from './signalTypes'

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
    retainCompletedOverflow: false,
    lastCompletionIndex: -1,
    lastCompletionId: ''
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
    if (
      message?.metadata?.message_kind === 'runtime_observation' ||
      message?.metadata?.messageKind === 'runtime_observation'
    ) {
      return false
    }
    if (message?.metadata?.error_type === 'SubAgentInterrupted') return true
    if (message?.metadata?.errorType === 'SubAgentInterrupted') return true
    if (message?.role !== 'user') return false
    if ((message.stepType || '').toLowerCase() !== 'observe') return false
    return removeSystemReminder(message.message || '').trim() === ''
  }

  const isCompletionReportMessage = message =>
    message?.role === 'assistant' &&
    (message?.metadata?.message_kind === 'completion_report' ||
      message?.metadata?.messageKind === 'completion_report')

  const isFinishTaskMessage = message => {
    const toolName = String(
      message?.metadata?.tool_name ||
        message?.metadata?.tool_call?.name ||
        message?.metadata?.tool_call?.function?.name ||
        ''
    ).toLowerCase()
    return toolName === 'complete_workflow_with_summary'
  }

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

    // Current backend tool observations always carry a terminal execution status.
    // The status-less branch only keeps compatibility with older persisted transcripts.
    return executionStatus ? executionStatus === 'completed' : true
  }

  const getMessageToolCallId = message =>
    String(message?.metadata?.tool_call_id || message?.metadata?.toolCallId || '')

  const filteredWorkflowMessages = computed(() =>
    (workflowStore.messages || []).filter(message => {
      if (isHiddenSystemObservation(message)) return false
      const messageWorkflowId = message?.sessionId || message?.session_id
      return !messageWorkflowId || messageWorkflowId === workflowStore.currentWorkflowId
    })
  )

  const getMessageSegmentId = message => {
    const rawSegmentId = message?.segment_id ?? message?.segmentId ?? message?.metadata?.segment_id
    const parsed = Number(rawSegmentId)
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null
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
      retainCompletedOverflow: false,
      lastCompletionIndex,
      lastCompletionId:
        lastCompletionIndex >= 0
          ? getMessageIdentity(messages[lastCompletionIndex], lastCompletionIndex)
          : ''
    }
  }

  const reconcileTaskWindow = messages => {
    const state = taskWindowState.value
    const workflowId = workflowStore.currentWorkflowId

    if (state.workflowId !== workflowId) {
      taskGroupCache.clear()
      acceptedTaskCompletionIds.clear()
      taskWindowState.value = {
        workflowId,
        initialized: false,
        completedGroups: [],
        activeMessages: [],
        retainCompletedOverflow: false,
        lastCompletionIndex: -1,
        lastCompletionId: ''
      }
    }

    if (!messages.length) {
      taskWindowState.value = {
        ...taskWindowState.value,
        activeMessages: [],
        retainCompletedOverflow: false
      }
      return
    }

    if (!taskWindowState.value.initialized) {
      initializeTaskWindow(messages)
      return
    }

    const currentState = taskWindowState.value
    if (currentState.lastCompletionIndex >= 0) {
      const boundaryMessage = messages[currentState.lastCompletionIndex]
      if (
        !boundaryMessage ||
        getMessageIdentity(boundaryMessage, currentState.lastCompletionIndex) !==
          currentState.lastCompletionId
      ) {
        initializeTaskWindow(messages)
        return
      }
    }

    const activeStartIndex = currentState.lastCompletionIndex + 1
    const activeTail = messages.slice(activeStartIndex)
    const tailGroups = buildTaskGroups(activeTail)
    const newlyCompletedGroups = tailGroups.filter(group => group.isCompleted)
    const activeGroup = tailGroups.find(group => !group.isCompleted)

    if (!newlyCompletedGroups.length) {
      currentState.activeMessages = activeTail
      if (activeTail.length > 0) {
        currentState.retainCompletedOverflow = false
      }
      return
    }

    const completedMessageCount = newlyCompletedGroups.reduce(
      (count, group) => count + group.messages.length,
      0
    )
    const nextLastCompletionIndex = activeStartIndex + completedMessageCount - 1
    currentState.completedGroups.push(...newlyCompletedGroups)
    currentState.activeMessages = activeGroup?.messages || []
    currentState.retainCompletedOverflow = !activeGroup
    currentState.lastCompletionIndex = nextLastCompletionIndex
    currentState.lastCompletionId = getMessageIdentity(
      messages[nextLastCompletionIndex],
      nextLastCompletionIndex
    )
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
    const completedGroupLimit =
      visibleCompletedTaskGroupCount.value + (state.retainCompletedOverflow ? 1 : 0)
    const visibleCompletedGroups = state.completedGroups.slice(
      -completedGroupLimit
    )
    const activeGroup = state.activeMessages.length
      ? {
          id: buildTaskGroupId(state.activeMessages),
          isCompleted: false,
          messages: state.activeMessages
        }
      : null

    return {
      groups: activeGroup ? [...visibleCompletedGroups, activeGroup] : visibleCompletedGroups,
      activeGroupId: activeGroup?.id || ''
    }
  })

  const hiddenCompletedTaskGroupCount = computed(() => {
    const visibleLimit =
      visibleCompletedTaskGroupCount.value +
      (taskWindowState.value.retainCompletedOverflow ? 1 : 0)
    return Math.max(0, taskWindowState.value.completedGroups.length - visibleLimit)
  })

  const enhanceRawMessages = rawMsgs => {
    if (!rawMsgs.length) return []
    const ledgerStateById = new Map(
      (workflowStore.toolList || []).map(tool => [tool.toolCallId, tool])
    )
    const subAgentProgressById = workflowStore.subAgentProgress || new Map()
    const toolStates = new Map() // tool_call_id -> { isFinal: bool, isRejected: bool, hasError: bool, isRunning: bool }
    const toolHasWaitingMsg = new Set() // tool_call_id that has an 'Awaiting' message
    const toolMessageIds = new Set() // tool_call_id with dedicated tool/user-observe messages
    const subAgentCompletions = new Map()
    const currentStatus = String(workflowStore.currentWorkflow?.status || '').toLowerCase()
    const isAwaitingUser =
      workflowStore.waitReason === WORKFLOW_WAIT_REASONS.USER_INPUT ||
      currentStatus === WORKFLOW_STATUSES.AWAITING_USER
    const askUserTailKinds = new Set(['ask_user_wait', 'ask_user_answered'])
    const tailRewindKind = String(workflowStore.currentWorkflow?.tailRewindKind || '').trim()
    const hasAskUserTail = askUserTailKinds.has(tailRewindKind)

    const isCriticalErrorText = content => /^critical error:/i.test(String(content || '').trim())

    const isMessageError = message => {
      const meta = message?.metadata || {}
      const content = removeSystemReminder(message?.message || '')
      return Boolean(
        message?.isError ||
          message?.is_error ||
          meta?.is_error ||
          message?.errorType ||
          message?.error_type ||
          meta?.error_type ||
          meta?.errorType ||
          isCriticalErrorText(content)
      )
    }

    const extractSubAgentTask = content => {
      if (!content || typeof content !== 'string') return ''
      const patterns = [
        /Task '([^']+)' has been spawned/i,
        /Sub-agent '([^']+)' has been started/i
      ]
      for (const pattern of patterns) {
        const match = content.match(pattern)
        if (match?.[1]) return match[1]
      }
      return ''
    }

    const parseSubAgentRunPayload = message => {
      const meta = message?.metadata || {}
      let parsed = {}
      try {
        parsed = JSON.parse(message?.message || '{}')
      } catch {
        parsed = {}
      }

      const taskId = meta.sub_agent_id || meta.subAgentId || parsed.task_id || parsed.taskId || ''
      const mode = meta.sub_agent_mode || meta.subAgentMode || parsed.mode || ''
      const task =
        meta.sub_agent_task ||
        meta.subAgentTask ||
        parsed.task ||
        extractSubAgentTask(parsed.message || message?.message || '')
      return {
        taskId,
        mode,
        task,
        agent:
          meta.sub_agent_name ||
          meta.subAgentName ||
          parsed.agent_name ||
          parsed.agentName ||
          ''
      }
    }

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

    const hasRealUserResponseAfterIndex = (list, startIndex) => {
      for (let i = startIndex + 1; i < list.length; i++) {
        const msg = list[i]
        if (msg?.role !== 'user') continue
        if (msg?.metadata?.queue_status === 'queued') continue
        const content = removeSystemReminder(msg.message || '').trim()
        if (!content) continue
        return true
      }
      return false
    }

    const buildSubAgentCard = message => {
      const meta = message?.metadata || {}
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
      const liveProgress = payload.taskId ? subAgentProgressById.get(payload.taskId) : null
      const completion = payload.taskId ? subAgentCompletions.get(payload.taskId) : null
      const completionResult = completion?.result || completion?.data?.result || {}
      const completionStatus =
        completion?.execution_status ||
        completion?.data?.execution_status ||
        completionResult.status ||
        liveProgress?.status ||
        meta.sub_agent_status ||
        meta.execution_status ||
        'running'
      const toolCallsCount =
        completionResult.tool_calls_count ??
        completion?.tool_calls_count ??
        completion?.data?.tool_calls_count ??
        liveProgress?.toolCallsCount ??
        liveProgress?.tool_calls_count ??
        0
      const currentContextTokens =
        completionResult.current_context_tokens ??
        completion?.current_context_tokens ??
        completion?.data?.current_context_tokens ??
        liveProgress?.currentContextTokens ??
        liveProgress?.current_context_tokens ??
        null
      const maxContextTokens =
        completionResult.max_context_tokens ??
        completion?.max_context_tokens ??
        completion?.data?.max_context_tokens ??
        liveProgress?.maxContextTokens ??
        liveProgress?.max_context_tokens ??
        null
      const resultContent =
        completionResult.result ||
        completionResult.error ||
        completion?.summary ||
        completion?.data?.summary ||
        ''
      const resultMarkdown = formatSubAgentResultMarkdown(resultContent)

      return {
        taskId: payload.taskId,
        agent: payload.agent || 'Sub-agent',
        task: payload.task || 'Delegated task',
        taskMarkdown: payload.task || 'Delegated task',
        mode: payload.mode || 'call',
        status: completionStatus,
        toolCallsCount,
        currentContextTokens,
        maxContextTokens,
        result: resultContent,
        resultMarkdown,
        hasResult: Boolean(resultContent)
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
          // Use approval_status as the primary indicator
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
          executionStatus === 'interrupted' ||
          executionStatus === 'rejected'
        ) {
          const isRejected = executionStatus === 'rejected'
          const isError =
            executionStatus === 'failed' || executionStatus === 'interrupted' || isMessageError(m)
          toolStates.set(id, { isFinal: true, isRejected, hasError: isError })
        } else if (approvalStatus === 'rejected') {
          // Final states
          const isError = isMessageError(m)
          toolStates.set(id, { isFinal: true, isRejected: true, hasError: isError })
        } else if (
          approvalStatus === 'approved' &&
          executionStatus !== 'approval_submitted' &&
          executionStatus !== 'running'
        ) {
          const isError = isMessageError(m)
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: isError })
        } else if (m.role === 'tool') {
          // Fallback: normal tool execution result (no approval flow)
          const isError = isMessageError(m)
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

          // Hide old waiting cards once the actual execution/rejection state has taken over.
          if ((state?.isFinal || state?.isRunning || isResolvedByLedger) && approvalStatus === 'pending') {
            if (toolHasWaitingMsg.has(id)) return false
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
      .map((message, idx) => {
        let toolDisplay
        try {
          toolDisplay = getToolDisplayInfo(message)
        } catch (error) {
          console.error('[Workflow] Failed to format tool message:', error, message)
          const fallbackTitle = t('chat.toolResult') || 'Result'
          toolDisplay = {
            title: fallbackTitle,
            summary: removeSystemReminder(message?.metadata?.summary || ''),
            isError: isMessageError(message),
            displayType: 'text',
            icon: 'tool',
            toolType: 'tool-system',
            action: fallbackTitle,
            target: '',
            hasStreamOutput: false,
            executionStatus: String(message?.metadata?.execution_status || '')
          }
        }
        const displayId = message.id || `msg_${message.role}_${message.stepIndex}_${idx}`

        let isRejected = false
        let isApproved = false

        // Check approval status from metadata (preferred method)
        const approvalStatus = message.metadata?.approval_status
        const executionStatus = message.metadata?.execution_status
        if (approvalStatus === 'rejected' || executionStatus === 'rejected') {
          isRejected = true
        } else if (executionStatus === 'interrupted') {
          isApproved = false
        } else if (
          executionStatus === 'approval_submitted' ||
          executionStatus === 'running'
        ) {
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
              const name = call?.function?.name || call?.name || ''
              const rawArgs = call?.function?.arguments || call?.arguments || {}
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
              const { icon, toolType, action, target } = formatToolTitle(name, args)
              const state = toolStates.get(call?.id)
              const ledgerState = ledgerStateById.get(call?.id)
              const isRejected =
                ledgerState?.status === 'rejected' || (!!state?.isFinal && !!state?.isRejected)
              const isRunning = ledgerState?.status === 'approved_running' || !!state?.isRunning
              const completionSummary =
                name === 'complete_workflow_with_summary' && typeof args.summary === 'string'
                  ? args.summary.trim()
                  : ''
              return {
                id: call?.id,
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
                      : undefined,
                  isRejected
                    ? 'User rejected'
                    : 'Executing...'
                )
              }
            })
            .filter(call => {
              // Tool lifecycle events can arrive before the persisted result message.
              // Keep the assistant card visible until its replacement actually exists.
              return !toolMessageIds.has(call.id)
            })
        }

        return {
          ...message,
          displayId,
          isError: isMessageError(message),
          toolDisplay,
          subAgentCard: buildSubAgentCard(message),
          pendingToolCalls,
          isRejected,
          isApproved
        }
      })
      .filter((m, index, list) => {
        if (m.metadata?.ui_visibility === 'hide') return false
        // Standard visibility logic
        if (m.role === 'tool') {
          const name =
            m.metadata?.tool_name ||
            m.metadata?.tool_call?.name ||
            m.metadata?.tool_call?.function?.name ||
            ''
          if (name === 'answer_user') return false
          if (
            name === 'ask_user' &&
            !isAwaitingUser &&
            !hasAskUserTail &&
            !hasRealUserResponseAfterIndex(list, index)
          ) {
            return false
          }
          if (
            ['approval_submitted', 'running'].includes(m.metadata?.execution_status) &&
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

  const enhancedMessages = computed(() => {
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

      if (group.isCompleted && group.id !== activeGroupId) {
        if (cachedEntry?.signature === signature) return cachedEntry.messages
      }

      const enhanced = reuseUnchangedEnhancedMessages(
        cachedEntry,
        enhanceRawMessages(group.messages)
      )
      taskGroupCache.set(group.id, {
        signature,
        messages: enhanced.messages,
        messageSignatures: enhanced.messageSignatures
      })
      return enhanced.messages
    })
  })

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
    if (
      message.metadata?.approval_status === 'pending' &&
      message.metadata?.execution_status !== 'approval_submitted' &&
      message.metadata?.execution_status !== 'running'
    ) {
      return true
    }
    const toolName =
      message.metadata?.tool_name ||
      message.metadata?.tool_call?.name ||
      message.metadata?.tool_call?.function?.name ||
      ''
    if (toolName === 'ask_user') return true
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
      skill: args => ({
        icon: resolveWorkflowToolIcon(name, 'skill'),
        toolType: 'tool-system',
        action: 'Activate Skill',
        target: args.skill || ''
      }),
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
        : executionStatus === 'approval_submitted' || executionStatus === 'running'
          ? 'running'
          : executionStatus === 'rejected'
            ? 'rejected'
            : executionStatus === 'completed'
              ? isError
                ? 'failed'
                : 'success'
              : executionStatus === 'failed' || executionStatus === 'interrupted'
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
      const hasPath = typeof payload.file_path === 'string' || typeof payload.path === 'string'
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
    if (
      meta.hide_approval_details &&
      (meta.execution_status === 'approval_submitted' ||
        meta.execution_status === 'running' ||
        meta.execution_status === 'interrupted')
    ) {
      return false
    }
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
          data.context_after_start_line || currentLineNew
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
          return name !== 'complete_workflow_with_summary' && name !== 'answer_user'
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
