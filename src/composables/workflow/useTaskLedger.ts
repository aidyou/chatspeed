import { ref, computed, type Ref } from 'vue'
import { getToolStatusSummary } from './toolDisplay'

export type ToolViewStatus =
  | 'pending'
  | 'approved_running'
  | 'rejected'
  | 'final_success'
  | 'final_error'

export interface ToolViewState {
  toolCallId: string
  toolName: string
  status: ToolViewStatus
  title: string
  summary: string
  arguments?: Record<string, any>
  result?: string
  errorType?: string
  approvalStatus?: 'pending' | 'approved' | 'rejected'
  createdAt: number
  updatedAt: number
  workflowId: string
  streamOutput?: string[]
  isExpanded?: boolean
}

export interface TaskLedgerState {
  tools: Map<string, ToolViewState>
  currentWorkflowId: string | null
  lastUpdated: number
}

export function useTaskLedger() {
  const ledgerMap = ref(new Map<string, TaskLedgerState>())
  const currentWorkflowId = ref<string | null>(null)

  const currentLedger = computed((): TaskLedgerState => {
    const id = currentWorkflowId.value
    if (!id) {
      return { tools: new Map(), currentWorkflowId: null, lastUpdated: 0 }
    }
    if (!ledgerMap.value.has(id)) {
      ledgerMap.value.set(id, {
        tools: new Map(),
        currentWorkflowId: id,
        lastUpdated: Date.now()
      })
    }
    return ledgerMap.value.get(id)!
  })

  const toolList = computed((): ToolViewState[] => {
    const tools = Array.from(currentLedger.value.tools.values())
    return tools.sort((a, b) => a.createdAt - b.createdAt)
  })

  const toolsByStatus = computed(() => {
    const list = toolList.value
    return {
      pending: list.filter(t => t.status === 'pending'),
      running: list.filter(t => t.status === 'approved_running'),
      completed: list.filter(t => t.status === 'final_success'),
      failed: list.filter(t => t.status === 'final_error' || t.status === 'rejected'),
      all: list
    }
  })

  const progressStats = computed(() => {
    const total = toolList.value.length
    const completed = toolsByStatus.value.completed.length
    const failed = toolsByStatus.value.failed.length
    const running = toolsByStatus.value.running.length
    const pending = toolsByStatus.value.pending.length
    const finished = completed + failed
    const percent = total > 0 ? Math.round((finished / total) * 100) : 0

    return { total, completed, failed, running, pending, finished, percent }
  })

  const setCurrentWorkflow = (workflowId: string | null) => {
    currentWorkflowId.value = workflowId
    if (workflowId && !ledgerMap.value.has(workflowId)) {
      ledgerMap.value.set(workflowId, {
        tools: new Map(),
        currentWorkflowId: workflowId,
        lastUpdated: Date.now()
      })
    }
  }

  const clearWorkflowLedger = (workflowId: string) => {
    ledgerMap.value.delete(workflowId)
    if (currentWorkflowId.value === workflowId) {
      currentWorkflowId.value = null
    }
  }

  const upsertTool = (state: Partial<ToolViewState> & { toolCallId: string }): ToolViewState => {
    const ledger = currentLedger.value
    const existing = ledger.tools.get(state.toolCallId)
    const now = Date.now()

    const next: ToolViewState = {
      toolCallId: state.toolCallId,
      toolName: state.toolName || existing?.toolName || 'unknown',
      status: state.status || existing?.status || 'pending',
      title: state.title || existing?.title || state.toolName || 'Tool',
      summary: state.summary ?? existing?.summary ?? 'Waiting...',
      arguments: state.arguments || existing?.arguments,
      result: state.result ?? existing?.result,
      errorType: state.errorType ?? existing?.errorType,
      approvalStatus: state.approvalStatus || existing?.approvalStatus || 'pending',
      createdAt: existing?.createdAt || now,
      updatedAt: now,
      workflowId: currentWorkflowId.value || existing?.workflowId || '',
      streamOutput: state.streamOutput || existing?.streamOutput || [],
      isExpanded: state.isExpanded ?? existing?.isExpanded ?? false
    }

    // Status priority: final_error/final_success > rejected > approved_running > pending
    if (existing) {
      const priority: Record<ToolViewStatus, number> = {
        'final_error': 4,
        'final_success': 4,
        'rejected': 3,
        'approved_running': 2,
        'pending': 1
      }

      const existingPriority = priority[existing.status] || 0
      const newPriority = priority[next.status] || 0

      if (existingPriority > newPriority && !state.status) {
        next.status = existing.status
      }
    }

    ledger.tools.set(state.toolCallId, next)
    ledger.lastUpdated = now

    return next
  }

  const markToolApproved = (toolCallId: string): ToolViewState | null => {
    const ledger = currentLedger.value
    const existing = ledger.tools.get(toolCallId)
    if (!existing) return null

    if (existing.status === 'final_success' || existing.status === 'final_error' || existing.status === 'rejected') {
      return existing
    }

    return upsertTool({
      toolCallId,
      status: 'approved_running',
      approvalStatus: 'approved',
      summary: getToolStatusSummary(
        existing.toolName,
        'running',
        existing.summary === 'Awaiting approval' ? 'Executing...' : existing.summary
      )
    })
  }

  const markToolRejected = (toolCallId: string): ToolViewState | null => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (!existing) return null

    if (existing.status === 'final_success' || existing.status === 'final_error') {
      return existing
    }

    return upsertTool({
      toolCallId,
      status: 'rejected',
      approvalStatus: 'rejected',
      summary: getToolStatusSummary(existing.toolName, 'rejected', 'User rejected')
    })
  }

  const finalizeTool = (
    toolCallId: string,
    success: boolean,
    result?: string,
    errorType?: string
  ): ToolViewState | null => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (!existing) return null

    return upsertTool({
      toolCallId,
      status: success ? 'final_success' : 'final_error',
      result,
      errorType,
      summary: success ? (result?.substring(0, 100) || 'Completed') : (errorType || 'Failed')
    })
  }

  const appendStreamOutput = (toolCallId: string, line: string): ToolViewState | null => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (!existing) return null

    const streamOutput = [...(existing.streamOutput || []), line]
    if (streamOutput.length > 100) {
      streamOutput.splice(0, streamOutput.length - 100)
    }

    return upsertTool({
      toolCallId,
      streamOutput,
      summary: line.trim().substring(0, 100) || existing.summary
    })
  }

  const toggleToolExpand = (toolCallId: string): void => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (existing) {
      upsertTool({
        toolCallId,
        isExpanded: !existing.isExpanded
      })
    }
  }

  const getTool = (toolCallId: string): ToolViewState | undefined => {
    return currentLedger.value.tools.get(toolCallId)
  }

  const getAllTools = (): ToolViewState[] => {
    return toolList.value
  }

  const hasToolsWithStatus = (status: ToolViewStatus | ToolViewStatus[]): boolean => {
    const statuses = Array.isArray(status) ? status : [status]
    return toolList.value.some(t => statuses.includes(t.status))
  }

  const clearCurrentLedger = () => {
    if (currentWorkflowId.value) {
      ledgerMap.value.delete(currentWorkflowId.value)
    }
  }

  const resetAllLedgers = () => {
    ledgerMap.value.clear()
    currentWorkflowId.value = null
  }

  return {
    currentWorkflowId,
    currentLedger,
    toolList,
    toolsByStatus,
    progressStats,

    setCurrentWorkflow,
    clearWorkflowLedger,
    upsertTool,
    markToolApproved,
    markToolRejected,
    finalizeTool,
    appendStreamOutput,
    toggleToolExpand,
    getTool,
    getAllTools,
    hasToolsWithStatus,
    clearCurrentLedger,
    resetAllLedgers
  }
}

export type TaskLedger = ReturnType<typeof useTaskLedger>
