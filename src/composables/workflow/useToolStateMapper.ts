/**
 * Message to Task Ledger State Mapping Convergence
 *
 * Core responsibility: Converge workflow messages to unified ToolViewState
 * Eliminate conflicting displays for the same tool_call_id
 */

import type { ToolViewState, ToolViewStatus } from './useTaskLedger'
import { isAutoExecuteWorkflowTool } from './toolApproval'
import {
  formatDisplayPath,
  getToolStatusSummary,
  normalizeShellCommandForDisplay,
  normalizeToolDisplayText
} from './toolDisplay'

/** Raw message interface */
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

/** Message metadata interface */
export interface MessageMetadata {
  tool_call_id?: string
  tool_call?: ToolCallInfo
  tool_calls?: ToolCallInfo[]
  tool_name?: string
  title?: string
  summary?: string
  approval_status?: 'pending' | 'approved' | 'rejected'
  execution_status?: 'pending_approval' | 'approval_submitted' | 'running' | 'completed' | 'failed' | 'interrupted' | 'rejected'
  is_error?: boolean
  arguments?: Record<string, any>
  display_type?: string
  hide_approval_details?: boolean
  queued_user_message_id?: string
  queue_status?: string
}

/** Tool call information */
export interface ToolCallInfo {
  id?: string
  name?: string
  function?: {
    name?: string
    arguments?: string | Record<string, any>
  }
  arguments?: string | Record<string, any>
}

/** Tool stream output map */
export interface ToolStreamMap {
  get(toolId: string): string[] | undefined
}

function isInternalTool(toolName: string): boolean {
  const name = String(toolName || '').toLowerCase()
  return [
    'answer_user',
    'ask_user',
    'complete_workflow',
    'submit_plan',
    'sub_agent_run',
    'sub_agent_output',
    'sub_agent_stop'
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
 * Extract tool call ID from message
 */
function extractToolCallId(message: RawMessage): string | null {
  const meta = message.metadata
  if (!meta) return null

  // Directly specified
  if (meta.tool_call_id) return meta.tool_call_id

  // Extract from tool_call
  if (meta.tool_call?.id) return meta.tool_call.id

  return null
}

/**
 * Extract tool name from message
 */
function extractToolName(message: RawMessage): string {
  const meta = message.metadata
  if (!meta) return 'unknown'

  // Directly specified
  if (meta.tool_name) return meta.tool_name

  // Extract from tool_call
  const toolCall = meta.tool_call
  if (toolCall) {
    return toolCall.name || toolCall.function?.name || 'unknown'
  }

  // Infer from title
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
 * Extract arguments from message
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
 * Determine the status for a message
 *
 * Status priority: final_error > final_success > rejected > approved_running > pending
 */
function determineStatus(message: RawMessage): ToolViewStatus | null {
  const meta = message.metadata
  if (!meta) return null

  const toolCallId = meta.tool_call_id
  if (!toolCallId) return null

  const executionStatus = meta.execution_status

  if (executionStatus === 'pending_approval') return 'pending'
  if (executionStatus === 'approval_submitted' || executionStatus === 'running') return 'approved_running'
  if (executionStatus === 'rejected') return 'rejected'
  if (executionStatus === 'failed' || executionStatus === 'interrupted') return 'final_error'
  if (executionStatus === 'completed') return 'final_success'

  // Check approval status
  const approvalStatus = meta.approval_status

  // If tool role message (execution result)
  if (message.role === 'tool') {
    if (approvalStatus === 'pending') return 'pending'
    if (approvalStatus === 'rejected') return 'rejected'
    const isError = message.isError || message.is_error || meta.is_error
    if (isError) return 'final_error'
    return 'final_success'
  }

  // If assistant message with tool_calls
  if (message.role === 'assistant' && meta.tool_calls?.length) {
    // Pending tool calls
    return 'pending'
  }

  // Determine by approval status
  if (approvalStatus === 'pending') return 'pending'
  if (approvalStatus === 'approved') return 'approved_running'
  if (approvalStatus === 'rejected') return 'rejected'

  return null
}

/**
 * Generate tool display title
 */
function generateTitle(toolName: string, args?: Record<string, any>): string {
  const formatters: Record<string, (args: Record<string, any>) => string> = {
    read_file: a => `Read ${formatDisplayPath(a.file_path || a.path || 'file')}`,
    write_file: a => `Write ${formatDisplayPath(a.file_path || a.path || 'file')}`,
    edit_file: a => `Edit ${formatDisplayPath(a.file_path || a.path || 'file')}`,
    list_dir: a => `List ${formatDisplayPath(a.path || a.dir || '.')}`,
    glob: a => {
      const path = formatDisplayPath(a.path || '')
      return path ? `Glob ${a.pattern || a.glob || ''} in ${path}` : `Glob ${a.pattern || a.glob || ''}`
    },
    grep: a => {
      const path = formatDisplayPath(a.path || '')
      return path ? `Grep "${a.pattern || a.query || ''}" in ${path}` : `Grep "${a.pattern || a.query || ''}"`
    },
    bash: a => `Run ${normalizeShellCommandForDisplay(a.command || '')}`,
    web_fetch: a => `Fetch ${a.url || ''}`,
    web_search: a => `Search "${a.query || ''}"`,
    todo_create: () => 'Create Todo',
    todo_update: () => 'Update Todo',
    sub_agent_run: () => 'Run Sub-agent',
    sub_agent_output: () => 'Get Sub-agent Output',
    sub_agent_stop: () => 'Stop Sub-agent',
    complete_workflow: () => 'Complete Workflow',
    ask_user: () => 'Ask User'
  }

  const formatter = formatters[toolName]
  if (formatter && args) {
    return normalizeToolDisplayText(formatter(args))
  }

  return toolName.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
}

/**
 * Derive tool view state from messages
 *
 * Convergence rules:
 * 1. Same tool_call_id only keeps one state
 * 2. Priority: final_error > final_success > rejected > approved_running > pending
 * 3. Stream output merged to corresponding tool state
 */
export function deriveToolViewState(
  messages: RawMessage[],
  toolStreams: ToolStreamMap,
  workflowId: string
): Map<string, ToolViewState> {
  const result = new Map<string, ToolViewState>()
  const now = Date.now()

  for (const message of messages) {
    const toolCallId = extractToolCallId(message)
    if (!toolCallId) {
      if (message.role === 'assistant' && message.metadata?.tool_calls?.length) {
        for (const call of message.metadata.tool_calls) {
          const id = call.id
          if (!id) continue

          const toolName = call.name || call.function?.name || 'unknown'
          if (isInternalTool(toolName)) continue
          const args = safeParseArguments(call.arguments || call.function?.arguments)

            const existing = result.get(id)
            if (!existing) {
              const autoExecute = isAutoExecuteWorkflowTool(toolName)
              const pendingSummary = getToolStatusSummary(
                toolName,
                autoExecute ? 'running' : 'pending',
                autoExecute ? 'Executing...' : 'Awaiting approval'
              )
              result.set(id, {
                toolCallId: id,
                toolName,
                status: autoExecute ? 'approved_running' : 'pending',
                title: generateTitle(toolName, args),
                summary: pendingSummary,
                ...(args ? { arguments: args } : {}),
                approvalStatus: autoExecute ? 'approved' : 'pending',
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

    // Status priority
    const priority: Record<ToolViewStatus, number> = {
      final_error: 4,
      final_success: 4,
      rejected: 3,
      approved_running: 2,
      pending: 1
    }

    const newPriority = priority[status]
    const existingPriority = existing ? priority[existing.status] : 0

    // Update only when new status priority >= existing priority
    if (!existing || newPriority >= existingPriority) {
      const title = normalizeToolDisplayText(
        toolName === 'bash' ? generateTitle(toolName, args) : meta.title || generateTitle(toolName, args)
      )
      const summary = getToolStatusSummary(
        toolName,
        status === 'pending'
          ? 'pending'
          : status === 'approved_running'
            ? 'running'
            : status === 'rejected'
              ? 'rejected'
              : status === 'final_success'
                ? 'success'
                : status === 'final_error'
                  ? 'failed'
                  : undefined,
        meta.summary || (status === 'rejected' ? 'User rejected' : 'Executing...')
      )

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

  // Second pass: merge stream output
  for (const [toolCallId, state] of result) {
    const streamLines = toolStreams.get(toolCallId)
    if (streamLines && streamLines.length > 0) {
      state.streamOutput = streamLines
      // Update summary with last stream line
      const lastLine = streamLines[streamLines.length - 1]?.trim()
      if (lastLine && state.status === 'approved_running') {
        state.summary = lastLine.substring(0, 100)
      }
    }
  }

  return result
}

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
    throw new Error(`Multiple entries for tool_call_id ${toolCallId}: ${foundCount} entries`)
  }
}

export function checkForConflicts(tools: Map<string, ToolViewState>): {
  hasConflict: boolean
  conflicts: string[]
} {
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
