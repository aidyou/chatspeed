import { computed, ref, watch } from 'vue'
import { invokeWrapper } from '@/libs/tauri'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'

const DEFAULT_CHILD_AGENT_LIMIT = 5
const DEFAULT_RUNNING_SUMMARY = 'Running'

const extractSubAgentIdFromMessage = message => {
  const meta = message?.metadata || {}
  if (meta.sub_agent_id || meta.subAgentId) return meta.sub_agent_id || meta.subAgentId
  if (meta.data?.sub_agent_id || meta.data?.subAgentId) {
    return meta.data.sub_agent_id || meta.data.subAgentId
  }
  return null
}

const normalizeChildPanelStatus = (status, isError = false) => {
  const normalized = String(status || '').toLowerCase()
  if (isError || ['failed', 'error', 'cancelled', 'interrupted'].includes(normalized)) {
    return 'failed'
  }
  if (['completed', 'success'].includes(normalized)) return 'success'
  if (['running', 'thinking', 'executing', 'waiting', 'pending'].includes(normalized)) {
    return 'running'
  }
  return 'pending'
}

const contextPercentFromProgress = progress => {
  const current = progress?.currentContextTokens ?? progress?.current_context_tokens
  const max = progress?.maxContextTokens ?? progress?.max_context_tokens
  if (typeof current !== 'number' || typeof max !== 'number' || max <= 0) return null
  return Math.min(100, Math.round((current / max) * 100))
}

const buildSubAgentProgressFromSnapshot = (id, snapshot, workflowStore, agentStore) => {
  const ctx = snapshot?.executionContext || {}
  const workflow = snapshot?.workflow || {}
  const snapshotMessages = Array.isArray(snapshot?.messages) ? snapshot.messages : []
  const latest = [...snapshotMessages]
    .reverse()
    .find(message => message?.role === 'assistant' || message?.role === 'tool')
  const latestMeta = latest?.metadata || {}
  const status = ctx.state || workflow.status || 'pending'

  return {
    subAgentId: id,
    parentSessionId:
      workflow.parentSessionId || workflow.parent_session_id || workflowStore.currentWorkflowId,
    agentName:
      workflow.agentName ||
      workflow.agent_name ||
      agentStore.agents.find(agent => agent.id === (workflow.agentId || workflow.agent_id))?.name ||
      null,
    task: workflow.userQuery || workflow.user_query || workflow.title || null,
    status,
    workflowState: workflow.status || status,
    waitReason: ctx.waitReason || ctx.wait_reason || workflow.waitReason || null,
    title: workflow.title || workflow.userQuery || id,
    summary: latestMeta.summary || latest?.message || '',
    toolCallsCount: snapshotMessages.filter(message => message?.role === 'tool').length,
    currentContextTokens: ctx.currentContextTokens ?? ctx.current_context_tokens ?? null,
    maxContextTokens: ctx.maxContextTokens ?? ctx.max_context_tokens ?? null,
    isError:
      latest?.isError ||
      latest?.is_error ||
      latestMeta.is_error ||
      ['failed', 'error', 'cancelled'].includes(String(status).toLowerCase()),
    updatedAtMs: Date.now()
  }
}

let sharedSubAgentSummaries = null

const createSharedSubAgentSummaries = () => {
  const workflowStore = useWorkflowStore()
  const agentStore = useAgentStore()
  const childSnapshotProgress = ref(new Map())
  const childAgentLimit = DEFAULT_CHILD_AGENT_LIMIT
  const messagesSource = computed(() => workflowStore.messages || [])
  const removeSystemReminder = content =>
    String(content || '')
      .replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '')
      .trim()

  const truncateSummary = (value, limit = 60) => {
    const text = removeSystemReminder(String(value || '')).trim()
    if (!text) return ''
    return text.length > limit ? `${text.slice(0, limit)}...` : text
  }

  const childSessionIdsFromSource = computed(() => {
    const ctx = workflowStore.currentWorkflow?.executionContext || {}
    const sessionsFromContext = ctx.subAgentSessions || ctx.sub_agent_sessions || []
    const waitingTaskId = ctx.waitingOnSubAgentId || ctx.waiting_on_sub_agent_id || null
    const sessionsFromMessages = (messagesSource.value || [])
      .map(message => extractSubAgentIdFromMessage(message))
      .filter(Boolean)
    const sessionsFromProgress = Array.from(workflowStore.subAgentProgress?.keys?.() || [])

    return Array.from(
      new Set(
        [
          waitingTaskId,
          ...(Array.isArray(sessionsFromContext) ? sessionsFromContext : []),
          ...sessionsFromMessages,
          ...sessionsFromProgress
        ].filter(Boolean)
      )
    )
  })

  const childSessionIds = computed(() =>
    Array.from(
      new Set([...childSessionIdsFromSource.value, ...Array.from(childSnapshotProgress.value.keys())])
    )
  )

  const refreshChildSnapshots = async () => {
    const ids = childSessionIdsFromSource.value.slice(-childAgentLimit)
    if (!ids.length) {
      childSnapshotProgress.value = new Map()
      return
    }

    const next = new Map()
    await Promise.all(
      ids.map(async id => {
        try {
          const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: id })
          next.set(id, buildSubAgentProgressFromSnapshot(id, snapshot, workflowStore, agentStore))
        } catch (error) {
          console.warn(`[Workflow] Failed to load child task snapshot ${id}:`, error)
        }
      })
    )

    if (!workflowStore.currentWorkflowId) return
    childSnapshotProgress.value = next
  }

  const childAgentSummariesAll = computed(() => {
    const ids = childSessionIds.value
    const messages = messagesSource.value || []
    if (!ids.length) return []

    return ids.map(id => {
      const ctx = workflowStore.currentWorkflow?.executionContext || {}
      const childWorkflow = workflowStore.workflows.find(workflow => workflow.id === id)
      const related = messages.filter(message => extractSubAgentIdFromMessage(message) === id)
      const last = related[related.length - 1]
      const lastIndex = last ? messages.lastIndexOf(last) : -1
      const eventProgress = workflowStore.subAgentProgress?.get?.(id)
      const snapshotProgress = childSnapshotProgress.value.get(id)
      const progress = {
        ...(snapshotProgress || {}),
        ...(eventProgress || {})
      }

      let status =
        (ctx.waitingOnSubAgentId || ctx.waiting_on_sub_agent_id) === id ? 'running' : 'pending'
      let summary = DEFAULT_RUNNING_SUMMARY
      let toolCalls = 0
      const workflowAgentName =
        childWorkflow?.agentName ||
        childWorkflow?.agent_name ||
        agentStore.agents.find(agent => agent.id === (childWorkflow?.agentId || childWorkflow?.agent_id))
          ?.name ||
        null
      let agentName = progress.agentName || progress.agent_name || workflowAgentName || 'Sub-agent'
      let task =
        progress.task || childWorkflow?.userQuery || childWorkflow?.user_query || childWorkflow?.title || id

      if (last) {
        const meta = last.metadata || {}
        const observationData = meta.data || {}
        const content = truncateSummary(last.message || '')
        if (content) summary = content
        agentName =
          meta.sub_agent_name ||
          meta.subAgentName ||
          observationData.sub_agent_name ||
          observationData.subAgentName ||
          progress.agentName ||
          progress.agent_name ||
          workflowAgentName ||
          agentName
        task =
          meta.sub_agent_task ||
          meta.subAgentTask ||
          observationData.sub_agent_task ||
          observationData.subAgentTask ||
          progress.task ||
          childWorkflow?.userQuery ||
          childWorkflow?.user_query ||
          childWorkflow?.title ||
          task
        const executionStatus =
          meta.execution_status || meta.sub_agent_status || observationData.execution_status || ''
        if (
          last.isError ||
          meta.is_error ||
          observationData.is_error ||
          executionStatus === 'failed' ||
          executionStatus === 'cancelled'
        ) {
          status = 'failed'
        } else if (meta.result || observationData.result || executionStatus === 'completed') {
          status = 'success'
        } else if (
          executionStatus === 'waiting' ||
          executionStatus === 'approval_submitted' ||
          executionStatus === 'running'
        ) {
          status = 'running'
        }
        if (meta.summary || observationData.summary) {
          summary = truncateSummary(meta.summary || observationData.summary)
        }
        const resultObj = meta.result || observationData.result
        if (resultObj && typeof resultObj === 'object') {
          toolCalls =
            resultObj.tool_calls_count ||
            resultObj.toolCallsCount ||
            resultObj.tool_calls ||
            resultObj.toolCalls ||
            0
        }
      }

      if (progress.subAgentId || progress.sub_agent_id) {
        status = normalizeChildPanelStatus(
          progress.status || progress.workflowState || progress.workflow_state,
          progress.isError || progress.is_error
        )
        toolCalls = progress.toolCallsCount ?? progress.tool_calls_count ?? toolCalls
        agentName = progress.agentName || progress.agent_name || agentName
        task = progress.task || task
        summary = truncateSummary(progress.summary) || summary
      }

      if (childWorkflow?.status) {
        const workflowStatus = String(childWorkflow.status).toLowerCase()
        if (workflowStatus === 'completed') status = 'success'
        if (['error', 'failed', 'cancelled'].includes(workflowStatus)) status = 'failed'
      }

      return {
        id,
        agentName,
        task,
        status,
        summary,
        toolCalls,
        contextPercent: contextPercentFromProgress(progress),
        waitReason: progress.waitReason || progress.wait_reason || childWorkflow?.waitReason || null,
        lastSeen: lastIndex >= 0 ? lastIndex : progress.updatedAtMs || progress.updated_at_ms || 0
      }
    })
  })

  const childAgentSummaries = computed(() =>
    [...childAgentSummariesAll.value]
      .sort((left, right) => right.lastSeen - left.lastSeen)
      .slice(0, childAgentLimit)
  )

  watch(
    () => workflowStore.currentWorkflowId,
    () => {
      childSnapshotProgress.value = new Map()
    }
  )

  watch(
    () => childSessionIdsFromSource.value.join('|'),
    () => {
      void refreshChildSnapshots()
    },
    { immediate: true }
  )

  return {
    childSnapshotProgress,
    childSessionIdsFromSource,
    childAgentSummariesAll,
    childAgentSummaries,
    childAgentTotalCount: computed(() => childSessionIds.value.length),
    refreshChildSnapshots
  }
}

export function useSubAgentSummaries() {
  if (!sharedSubAgentSummaries) {
    sharedSubAgentSummaries = createSharedSubAgentSummaries()
  }
  return sharedSubAgentSummaries
}
