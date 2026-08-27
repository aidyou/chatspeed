import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invokeWrapper } from '@/libs/tauri'
import { deriveToolViewState } from '@/composables/workflow/useToolStateMapper'
import { hydrateWorkflowSession } from '@/composables/workflow/workflowSessionHydration.js'
import {
  appendMissingPendingToolMessages,
  deriveInlinePendingApprovals,
  normalizeExecutionContextForApproval,
  resolveExecutionContextPendingTool,
  upsertExecutionContextPendingTool
} from '@/stores/workflowApprovalRecovery.js'

const normalizeSnapshotMessage = (message, sessionId) => {
  const normalized = {
    ...message,
    id: message?.id ?? message?.persistedMessageId ?? message?.message_id ?? null,
    sessionId: message?.sessionId || message?.session_id || sessionId,
    messageKind: message?.messageKind || message?.message_kind || 'message',
    messageSubtype: message?.messageSubtype || message?.message_subtype || null
  }

  if (typeof normalized.metadata === 'string') {
    try {
      normalized.metadata = JSON.parse(normalized.metadata)
    } catch {
      normalized.metadata = {}
    }
  }
  if (normalized.metadata && typeof normalized.metadata === 'object') {
    normalized.metadata = {
      ...normalized.metadata,
      ui_visibility: normalized.metadata.ui_visibility ?? normalized.metadata.uiVisibility,
      message_kind: normalized.metadata.message_kind ?? normalized.metadata.messageKind,
      error_type: normalized.metadata.error_type ?? normalized.metadata.errorType
    }
  }
  return normalized
}

const normalizeConfirmPendingTool = payload => ({
  toolCallId: payload.id,
  toolName: payload.tool_name || payload.action || '',
  arguments: payload.arguments ?? null,
  details: payload.details ?? null,
  displayType: payload.display_type || ''
})

const buildPendingMessage = (sessionId, payload) =>
  appendMissingPendingToolMessages({
    messages: [],
    sessionId,
    executionContext: { pendingTools: [normalizeConfirmPendingTool(payload)] }
  })[0]

export function useWorkflowSessionMessages({ sessionId, agentRole }) {
  const messages = ref([])
  const workflow = ref(null)
  const isLoadingMessages = ref(false)
  const hiddenEarlierMessageCount = ref(0)
  const toolStreams = ref(new Map())
  const taskCompletionRevision = ref(0)
  const lastTaskCompletion = ref(null)
  const chatState = ref({ content: '', reasoning: '', reasoningStatus: 'idle', blocks: [], retryInfo: null })
  const compression = ref({ isCompressing: false, message: '' })
  const loadRevision = ref(0)
  let unlisten = null

  const executionContext = computed(() => workflow.value?.executionContext || {})
  const isRunning = computed(() => ['running', 'thinking', 'executing'].includes(String(workflow.value?.status || '').toLowerCase()))
  const waitReason = computed(() => workflow.value?.waitReason || workflow.value?.wait_reason || executionContext.value?.waitReason || '')
  const toolList = computed(() =>
    Array.from(
      deriveToolViewState(messages.value, { get: id => toolStreams.value.get(id) }, sessionId.value).values()
    ).sort((left, right) => left.createdAt - right.createdAt)
  )
  const pendingApprovals = computed(() => deriveInlinePendingApprovals({
    currentWorkflowId: sessionId.value,
    workflowTitle: workflow.value?.title || '',
    status: workflow.value?.status || '',
    waitReason: waitReason.value,
    messages: messages.value,
    executionContext: executionContext.value,
    approvalWaitingStatuses: ['awaiting_approval', 'awaiting_auto_approval']
  }))
  const pendingApprovalIds = computed(() => pendingApprovals.value.map(entry => entry.id))

  const addMessage = payload => {
    const normalized = normalizeSnapshotMessage({
      persistedMessageId: payload.message_id,
      role: payload.role,
      message: payload.content,
      reasoning: payload.reasoning,
      stepType: payload.step_type,
      stepIndex: payload.step_index,
      isError: payload.is_error,
      errorType: payload.error_type,
      metadata: payload.metadata
    }, sessionId.value)
    messages.value = [...messages.value, normalized]
    chatState.value = { content: '', reasoning: '', reasoningStatus: 'idle', blocks: [], retryInfo: null }
  }

  const applyEvent = payload => {
    if (!payload || !sessionId.value) return
    if (payload.type === 'message') {
      addMessage(payload)
    } else if (payload.type === 'chunk') {
      chatState.value = { ...chatState.value, content: `${chatState.value.content}${payload.content || ''}` }
    } else if (payload.type === 'reasoning_chunk') {
      chatState.value = { ...chatState.value, reasoning: `${chatState.value.reasoning}${payload.content || ''}`, reasoningStatus: 'streaming' }
    } else if (payload.type === 'confirm') {
      workflow.value = {
        ...(workflow.value || {}),
        executionContext: upsertExecutionContextPendingTool(executionContext.value, normalizeConfirmPendingTool(payload))
      }
      const pendingMessage = buildPendingMessage(sessionId.value, payload)
      if (pendingMessage) messages.value = [...messages.value, pendingMessage]
    } else if (payload.type === 'approval_resolved' || payload.type === 'tool_started' || payload.type === 'tool_completed' || payload.type === 'tool_failed') {
      const toolCallId = payload.tool_call_id
      workflow.value = {
        ...(workflow.value || {}),
        executionContext: resolveExecutionContextPendingTool(executionContext.value, toolCallId)
      }
    } else if (payload.type === 'state') {
      workflow.value = { ...(workflow.value || {}), status: payload.state, waitReason: payload.wait_reason || null }
      if (!['thinking', 'executing'].includes(String(payload.state || '').toLowerCase())) {
        chatState.value = { content: '', reasoning: '', reasoningStatus: 'idle', blocks: [], retryInfo: null }
      }
    } else if (payload.type === 'tool_stream') {
      const next = new Map(toolStreams.value)
      next.set(payload.tool_id, [...(next.get(payload.tool_id) || []), payload.output])
      toolStreams.value = next
    } else if (payload.type === 'compression_status') {
      compression.value = { isCompressing: payload.is_compressing === true, message: payload.message || '' }
    } else if (payload.type === 'task_completed') {
      taskCompletionRevision.value += 1
      lastTaskCompletion.value = { sessionId: sessionId.value, toolCallId: payload.tool_call_id }
    }
  }

  const hydrateChildSession = async () => {
    const targetSessionId = sessionId.value
    if (!targetSessionId) return
    const revision = ++loadRevision.value
    isLoadingMessages.value = true
    try {
      const { stop, applied } = await hydrateWorkflowSession({
        registerListener: handleEvent => listen(`workflow://event/${targetSessionId}`, event => {
          if (targetSessionId === sessionId.value) handleEvent(event.payload)
        }),
        fetchSnapshot: () => invokeWrapper('get_workflow_snapshot', { sessionId: targetSessionId }),
        applySnapshot: snapshot => {
          const context = normalizeExecutionContextForApproval(snapshot.executionContext)
          workflow.value = {
            ...(snapshot.workflow || {}),
            id: targetSessionId,
            executionContext: context
          }
          messages.value = appendMissingPendingToolMessages({
            messages: (snapshot.messages || []).map(message => normalizeSnapshotMessage(message, targetSessionId)),
            sessionId: targetSessionId,
            executionContext: context
          })
          hiddenEarlierMessageCount.value = Number(snapshot.hiddenEarlierMessageCount) || 0
        },
        applyEvent,
        isCurrent: () => revision === loadRevision.value && targetSessionId === sessionId.value,
        onListenerRegistered: stop => {
          if (revision === loadRevision.value && targetSessionId === sessionId.value) unlisten = stop
          else stop()
        }
      })
      if (!applied || targetSessionId !== sessionId.value) {
        stop()
        return
      }
      unlisten = stop
    } finally {
      if (revision === loadRevision.value) isLoadingMessages.value = false
    }
  }

  watch(sessionId, async () => {
    unlisten?.()
    unlisten = null
    messages.value = []
    workflow.value = null
    toolStreams.value = new Map()
    if (agentRole.value !== 'child') return
    await hydrateChildSession()
  }, { immediate: true })

  onBeforeUnmount(() => unlisten?.())

  const source = reactive({
    get messages() { return messages.value },
    get currentWorkflowId() { return sessionId.value },
    get currentWorkflow() { return workflow.value },
    get workflows() { return agentRole.value === 'child' ? [] : [] },
    get toolList() { return toolList.value },
    get subAgentProgress() { return new Map() },
    get taskCompletionRevision() { return taskCompletionRevision.value },
    get lastTaskCompletion() { return lastTaskCompletion.value },
    get hiddenEarlierMessageCount() { return hiddenEarlierMessageCount.value },
    getToolStream: toolCallId => toolStreams.value.get(toolCallId) || []
  })

  return {
    source,
    workflow,
    isLoadingMessages,
    isRunning,
    waitReason,
    chatState,
    isCompressing: computed(() => compression.value.isCompressing),
    compressionMessage: computed(() => compression.value.message),
    pendingApprovals,
    pendingApprovalIds,
    loadSnapshot: hydrateChildSession
  }
}
