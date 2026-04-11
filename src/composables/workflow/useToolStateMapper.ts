/**
 * 消息到任务账本状态映射收敛
 * 
 * 核心职责：将 workflow messages 收敛到统一的 ToolViewState
 * 消除多条消息对同一 tool_call_id 的冲突展示
 */

import type { ToolViewState, ToolViewStatus } from './useTaskLedger'

/** 原始消息接口 */
export interface RawMessage {
  id?: string | number
  role: 'user' | 'assistant' | 'tool' | 'system'
  message?: string
  metadata?: MessageMetadata
  stepType?: string
  isError?: boolean
  is_error?: boolean
  createdAt?: number
  stepIndex?: number
}

/** 消息元数据接口 */
export interface MessageMetadata {
  tool_call_id?: string
  tool_call?: ToolCallInfo
  tool_calls?: ToolCallInfo[]
  tool_name?: string
  title?: string
  summary?: string
  approval_status?: 'pending' | 'approved' | 'rejected'
  execution_status?: 'pending_approval' | 'running' | 'completed' | 'failed' | 'rejected'
  is_error?: boolean
  arguments?: Record<string, any>
  display_type?: string
  hide_approval_details?: boolean
  queued_user_message_id?: string
  queue_status?: string
}

/** 工具调用信息 */
export interface ToolCallInfo {
  id?: string
  name?: string
  function?: {
    name?: string
    arguments?: string | Record<string, any>
  }
  arguments?: string | Record<string, any>
}

/** 流式输出映射 */
export interface ToolStreamMap {
  get(toolId: string): string[] | undefined
}

function isInternalTool(toolName: string): boolean {
  const name = String(toolName || '').toLowerCase()
  return [
    'answer_user',
    'ask_user',
    'finish_task',
    'submit_plan',
    'task',
    'task_output',
    'task_stop'
  ].includes(name)
}

function safeParseArguments(raw: unknown): Record<string, any> {
  if (!raw) return {}
  if (typeof raw === 'string') {
    try {
      const parsed = JSON.parse(raw)
      return parsed && typeof parsed === 'object' ? parsed : {}
    } catch {
      return {}
    }
  }
  return raw && typeof raw === 'object' ? (raw as Record<string, any>) : {}
}

/**
 * 从消息中提取工具调用ID
 */
function extractToolCallId(message: RawMessage): string | null {
  const meta = message.metadata
  if (!meta) return null

  // 直接指定
  if (meta.tool_call_id) return meta.tool_call_id

  // 从 tool_call 提取
  if (meta.tool_call?.id) return meta.tool_call.id

  return null
}

/**
 * 从消息中提取工具名称
 */
function extractToolName(message: RawMessage): string {
  const meta = message.metadata
  if (!meta) return 'unknown'

  // 直接指定
  if (meta.tool_name) return meta.tool_name

  // 从 tool_call 提取
  const toolCall = meta.tool_call
  if (toolCall) {
    return toolCall.name || toolCall.function?.name || 'unknown'
  }

  // 从 title 推断
  if (meta.title) {
    const title = meta.title.toLowerCase()
    if (title.includes('read')) return 'read_file'
    if (title.includes('write')) return 'write_file'
    if (title.includes('edit')) return 'edit_file'
    if (title.includes('list')) return 'list_dir'
    if (title.includes('bash')) return 'bash'
    if (title.includes('grep')) return 'grep'
    if (title.includes('glob')) return 'glob'
    if (title.includes('web')) return 'web_fetch'
    if (title.includes('search')) return 'web_search'
  }

  return 'unknown'
}

/**
 * 从消息中提取参数
 */
function extractArguments(message: RawMessage): Record<string, any> | undefined {
  const meta = message.metadata
  if (!meta?.tool_call) return undefined

  const toolCall = meta.tool_call
  let args: any = toolCall.arguments || toolCall.function?.arguments

  if (typeof args === 'string') {
    try {
      args = JSON.parse(args)
    } catch {
      return undefined
    }
  }

  return args
}

/**
 * 确定消息对应的状态
 * 
 * 状态优先级：final_error > final_success > rejected > approved_running > pending
 */
function determineStatus(message: RawMessage): ToolViewStatus | null {
  const meta = message.metadata
  if (!meta) return null

  const toolCallId = meta.tool_call_id
  if (!toolCallId) return null

  const executionStatus = meta.execution_status

  if (executionStatus === 'pending_approval') return 'pending'
  if (executionStatus === 'running') return 'approved_running'
  if (executionStatus === 'rejected') return 'rejected'
  if (executionStatus === 'failed') return 'final_error'
  if (executionStatus === 'completed') return 'final_success'

  // 检查审批状态
  const approvalStatus = meta.approval_status

  // 如果是 tool 角色消息（执行结果）
  if (message.role === 'tool') {
    if (approvalStatus === 'pending') return 'pending'
    if (approvalStatus === 'approved') return 'approved_running'
    const isError = message.isError || message.is_error || meta.is_error
    if (isError) return 'final_error'

    // 如果审批状态是 rejected，保留拒绝状态
    if (approvalStatus === 'rejected') return 'rejected'

    // 其他情况视为成功完成
    return 'final_success'
  }

  // 如果是 assistant 消息的 tool_calls
  if (message.role === 'assistant' && meta.tool_calls?.length) {
    // Pending tool calls
    return 'pending'
  }

  // 根据审批状态判断
  if (approvalStatus === 'pending') return 'pending'
  if (approvalStatus === 'approved') return 'approved_running'
  if (approvalStatus === 'rejected') return 'rejected'

  return null
}

/**
 * 生成工具显示标题
 */
function generateTitle(toolName: string, args?: Record<string, any>): string {
  const formatters: Record<string, (args: Record<string, any>) => string> = {
    read_file: (a) => `Read ${a.file_path || a.path || 'file'}`,
    write_file: (a) => `Write ${a.file_path || a.path || 'file'}`,
    edit_file: (a) => `Edit ${a.file_path || a.path || 'file'}`,
    list_dir: (a) => `List ${a.path || a.dir || '.'}`,
    glob: (a) => `Glob ${a.pattern || a.glob || ''}`,
    grep: (a) => `Grep "${a.pattern || a.query || ''}"`,
    bash: (a) => `Bash: ${(a.command || '').substring(0, 40)}`,
    web_fetch: (a) => `Fetch ${a.url || ''}`,
    web_search: (a) => `Search "${a.query || ''}"`,
    todo_create: () => 'Create Todo',
    todo_update: () => 'Update Todo',
    finish_task: () => 'Finish Task',
    ask_user: () => 'Ask User'
  }

  const formatter = formatters[toolName]
  if (formatter && args) {
    return formatter(args)
  }

  return toolName.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
}

/**
 * 核心函数：从消息列表推导任务账本状态
 * 
 * 收敛规则：
 * 1. 同一 tool_call_id 只保留一个状态
 * 2. 优先级：final_error > final_success > rejected > approved_running > pending
 * 3. stream 输出合并到对应工具状态
 */
export function deriveToolViewState(
  messages: RawMessage[],
  toolStreams: ToolStreamMap,
  workflowId: string
): Map<string, ToolViewState> {
  const result = new Map<string, ToolViewState>()
  const now = Date.now()

  // 第一遍扫描：收集所有工具相关信息
  for (const message of messages) {
    const toolCallId = extractToolCallId(message)
    if (!toolCallId) {
      // 检查是否是包含 tool_calls 的 assistant 消息
      if (message.role === 'assistant' && message.metadata?.tool_calls?.length) {
        for (const call of message.metadata.tool_calls) {
          const id = call.id
          if (!id) continue

          const toolName = call.name || call.function?.name || 'unknown'
          if (isInternalTool(toolName)) continue
          const args = safeParseArguments(call.arguments || call.function?.arguments)

          const existing = result.get(id)
          if (!existing) {
            result.set(id, {
              toolCallId: id,
              toolName,
              status: 'pending',
              title: generateTitle(toolName, args),
              summary: 'Awaiting approval',
              arguments: args,
              createdAt: message.createdAt || now,
              updatedAt: now,
              workflowId,
              streamOutput: [],
              isExpanded: false
            })
          }
        }
      }
      continue
    }

    const status = determineStatus(message)
    if (!status) continue

    const toolName = extractToolName(message)
    if (isInternalTool(toolName)) continue
    const args = extractArguments(message)
    const meta = message.metadata || {}

    const existing = result.get(toolCallId)

    // 状态优先级
    const priority: Record<ToolViewStatus, number> = {
      'final_error': 4,
      'final_success': 4,
      'rejected': 3,
      'approved_running': 2,
      'pending': 1
    }

    const newPriority = priority[status]
    const existingPriority = existing ? priority[existing.status] : 0

    // 只有当新状态优先级 >= 现有状态时，才更新
    if (!existing || newPriority >= existingPriority) {
      const title = meta.title || generateTitle(toolName, args)
      const summary = meta.summary || (status === 'rejected' ? 'User rejected' : 'Executing...')

      result.set(toolCallId, {
        toolCallId,
        toolName,
        status,
        title,
        summary,
        arguments: args || existing?.arguments,
        result: message.role === 'tool' ? message.message : existing?.result,
        errorType: meta.is_error ? 'execution_error' : existing?.errorType,
        approvalStatus: meta.approval_status || existing?.approvalStatus || 'pending',
        createdAt: existing?.createdAt || message.createdAt || now,
        updatedAt: now,
        workflowId,
        streamOutput: toolStreams.get(toolCallId) || existing?.streamOutput || [],
        isExpanded: existing?.isExpanded || false
      })
    }
  }

  // 第二遍：合并流式输出
  for (const [toolCallId, state] of result) {
    const streamLines = toolStreams.get(toolCallId)
    if (streamLines && streamLines.length > 0) {
      state.streamOutput = streamLines
      // 更新摘要为最后一条流式输出
      const lastLine = streamLines[streamLines.length - 1]?.trim()
      if (lastLine && state.status === 'approved_running') {
        state.summary = lastLine.substring(0, 100)
      }
    }
  }

  return result
}

/**
 * 状态收敛断言（用于测试）
 * 验证同一 tool_call_id 不会产生冲突状态
 */
export function assertNoConflictingStates(
  tools: Map<string, ToolViewState>,
  toolCallId: string
): void {
  let foundCount = 0
  let foundStatus: ToolViewStatus | null = null

  for (const [id, state] of tools) {
    if (id === toolCallId) {
      foundCount++
      if (foundStatus && foundStatus !== state.status) {
        throw new Error(
          `Conflicting states for tool_call_id ${toolCallId}: ` +
          `${foundStatus} vs ${state.status}`
        )
      }
      foundStatus = state.status
    }
  }

  if (foundCount > 1) {
    throw new Error(
      `Multiple entries for tool_call_id ${toolCallId}: ${foundCount} entries`
    )
  }
}

/**
 * 检查工具列表中是否存在状态冲突
 */
export function checkForConflicts(
  tools: Map<string, ToolViewState>
): { hasConflict: boolean; conflicts: string[] } {
  const conflicts: string[] = []
  const seen = new Set<string>()

  for (const [toolCallId] of tools) {
    if (seen.has(toolCallId)) {
      conflicts.push(toolCallId)
    }
    seen.add(toolCallId)
  }

  return {
    hasConflict: conflicts.length > 0,
    conflicts
  }
}
