/**
 * Task Ledger - 统一任务账本模型
 * 
 * 阶段9核心实现：建立 tool_call_id 统一视图模型，避免多轨状态冲突
 * 
 * 状态机：
 * pending → approved_running → final_success
 *    ↓           ↓
 * rejected    final_error
 */

import { ref, computed, type Ref } from 'vue'

/** 工具视图状态枚举 */
export type ToolViewStatus =
  | 'pending'           // 等待审批
  | 'approved_running'  // 已批准，执行中
  | 'rejected'          // 已拒绝
  | 'final_success'     // 执行成功完成
  | 'final_error'       // 执行失败

/** 工具视图状态接口 */
export interface ToolViewState {
  /** 唯一标识 */
  toolCallId: string
  /** 工具名称 */
  toolName: string
  /** 当前状态 */
  status: ToolViewStatus
  /** 显示标题 */
  title: string
  /** 摘要说明 */
  summary: string
  /** 工具参数 */
  arguments?: Record<string, any>
  /** 执行结果 */
  result?: string
  /** 错误类型 */
  errorType?: string
  /** 审批状态 */
  approvalStatus?: 'pending' | 'approved' | 'rejected'
  /** 创建时间 */
  createdAt: number
  /** 更新时间 */
  updatedAt: number
  /** 所属 workflow */
  workflowId: string
  /** 流式输出内容（仅执行中） */
  streamOutput?: string[]
  /** 是否折叠（UI状态） */
  isExpanded?: boolean
}

/** 任务账本状态 */
export interface TaskLedgerState {
  /** tool_call_id -> ToolViewState 映射 */
  tools: Map<string, ToolViewState>
  /** 当前 workflow ID */
  currentWorkflowId: string | null
  /** 最后更新时间 */
  lastUpdated: number
}

/**
 * 创建任务账本 composable
 */
export function useTaskLedger() {
  // 内部状态 - 按 workflow 隔离存储
  const ledgerMap = ref(new Map<string, TaskLedgerState>())
  const currentWorkflowId = ref<string | null>(null)

  // 当前激活的任务账本
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

  // 当前工具列表（按更新时间排序）
  const toolList = computed((): ToolViewState[] => {
    const tools = Array.from(currentLedger.value.tools.values())
    return tools.sort((a, b) => a.createdAt - b.createdAt)
  })

  // 按状态分组的工具
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

  // 进度统计
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

  /**
   * 设置当前 workflow（切换会话时调用）
   */
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

  /**
   * 清理指定 workflow 的账本数据
   */
  const clearWorkflowLedger = (workflowId: string) => {
    ledgerMap.value.delete(workflowId)
    if (currentWorkflowId.value === workflowId) {
      currentWorkflowId.value = null
    }
  }

  /**
   * 创建或更新工具状态（核心 reducer 入口）
   */
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

    // 状态优先级收敛（防止冲突）
    // final_error/final_success > rejected > approved_running > pending
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

      // 如果现有状态优先级更高，保留现有状态（除非明确强制更新）
      if (existingPriority > newPriority && !state.status) {
        next.status = existing.status
      }
    }

    ledger.tools.set(state.toolCallId, next)
    ledger.lastUpdated = now

    return next
  }

  /**
   * 标记工具为已批准并执行中
   */
  const markToolApproved = (toolCallId: string): ToolViewState | null => {
    const ledger = currentLedger.value
    const existing = ledger.tools.get(toolCallId)
    if (!existing) return null

    // 如果已经是终态，不再更新
    if (existing.status === 'final_success' || existing.status === 'final_error' || existing.status === 'rejected') {
      return existing
    }

    return upsertTool({
      toolCallId,
      status: 'approved_running',
      approvalStatus: 'approved',
      summary: existing.summary === 'Awaiting approval' ? 'Executing...' : existing.summary
    })
  }

  /**
   * 标记工具为已拒绝
   */
  const markToolRejected = (toolCallId: string): ToolViewState | null => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (!existing) return null

    // 如果已经是终态，不再更新
    if (existing.status === 'final_success' || existing.status === 'final_error') {
      return existing
    }

    return upsertTool({
      toolCallId,
      status: 'rejected',
      approvalStatus: 'rejected',
      summary: 'User rejected'
    })
  }

  /**
   * 标记工具执行完成
   */
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

  /**
   * 追加流式输出
   */
  const appendStreamOutput = (toolCallId: string, line: string): ToolViewState | null => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (!existing) return null

    const streamOutput = [...(existing.streamOutput || []), line]
    // Keep only latest 100 lines
    if (streamOutput.length > 100) {
      streamOutput.splice(0, streamOutput.length - 100)
    }

    return upsertTool({
      toolCallId,
      streamOutput,
      summary: line.trim().substring(0, 100) || existing.summary
    })
  }

  /**
   * 切换工具展开状态
   */
  const toggleToolExpand = (toolCallId: string): void => {
    const existing = currentLedger.value.tools.get(toolCallId)
    if (existing) {
      upsertTool({
        toolCallId,
        isExpanded: !existing.isExpanded
      })
    }
  }

  /**
   * 根据 tool_call_id 获取工具状态
   */
  const getTool = (toolCallId: string): ToolViewState | undefined => {
    return currentLedger.value.tools.get(toolCallId)
  }

  /**
   * 获取所有工具（按创建时间排序）
   */
  const getAllTools = (): ToolViewState[] => {
    return toolList.value
  }

  /**
   * 检查是否存在指定状态的工具
   */
  const hasToolsWithStatus = (status: ToolViewStatus | ToolViewStatus[]): boolean => {
    const statuses = Array.isArray(status) ? status : [status]
    return toolList.value.some(t => statuses.includes(t.status))
  }

  /**
   * 清理当前会话的所有数据
   */
  const clearCurrentLedger = () => {
    if (currentWorkflowId.value) {
      ledgerMap.value.delete(currentWorkflowId.value)
    }
  }

  /**
   * 重置整个账本（用于测试或登出）
   */
  const resetAllLedgers = () => {
    ledgerMap.value.clear()
    currentWorkflowId.value = null
  }

  return {
    // State
    currentWorkflowId,
    currentLedger,
    toolList,
    toolsByStatus,
    progressStats,

    // Actions
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
