import { ref, computed } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
import { resolveWorkflowToolIcon } from './toolIcons'
import { isAutoExecuteWorkflowTool } from './toolApproval'
import { useI18n } from 'vue-i18n'
import * as Diff from 'diff'

/**
 * Composable for managing message processing and display
 * Handles enhanced messages, tool formatting, and expansion states
 */
export function useWorkflowMessages() {
  const { t } = useI18n()
  const workflowStore = useWorkflowStore()

  const expandedMessages = ref(new Set())
  const expandedReasonings = ref(new Set())

  // Compute last assistant message for streaming state detection
  const lastAssistantMessage = computed(() => {
    return enhancedMessages.value.filter(m => m.role === 'assistant').pop()
  })

  // Enhanced messages with pre-calculated display info
  const enhancedMessages = computed(() => {
    if (!workflowStore.messages || workflowStore.messages.length === 0) return []

    const rawMsgs = workflowStore.messages
    const toolStates = new Map() // tool_call_id -> { isFinal: bool, isRejected: bool, hasError: bool, isRunning: bool }
    const toolHasWaitingMsg = new Set() // tool_call_id that has an 'Awaiting' message
    const toolMessageIds = new Set() // tool_call_id with dedicated tool/user-observe messages
    const subAgentCompletions = new Map()

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

    const buildSubAgentCard = message => {
      const meta = message?.metadata || {}
      if (String(meta.tool_name || '').toLowerCase() !== 'sub_agent_run') return null

      const payload = parseSubAgentRunPayload(message)
      const completion = payload.taskId ? subAgentCompletions.get(payload.taskId) : null
      const completionResult = completion?.result || completion?.data?.result || {}
      const completionStatus =
        completion?.execution_status ||
        completion?.data?.execution_status ||
        completionResult.status ||
        meta.sub_agent_status ||
        meta.execution_status ||
        'running'
      const resultContent =
        completionResult.result ||
        completionResult.error ||
        completion?.summary ||
        completion?.data?.summary ||
        ''

      return {
        taskId: payload.taskId,
        agent: payload.agent || 'Sub-agent',
        task: payload.task || 'Delegated task',
        taskMarkdown: payload.task || 'Delegated task',
        mode: payload.mode || 'call',
        status: completionStatus,
        result: resultContent,
        resultMarkdown: resultContent,
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

        if (m.role === 'tool' || (m.role === 'user' && m.stepType === 'observe')) {
          toolMessageIds.add(id)
        }

        // Use approval_status as the primary indicator
        if (executionStatus === 'pending_approval' || approvalStatus === 'pending') {
          toolHasWaitingMsg.add(id)
        } else if (executionStatus === 'running') {
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

          // If there's a final result (approved, rejected, or executed)
          if (state?.isFinal) {
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
      .map((message, idx) => {
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
              return {
                id: call.id,
                icon,
                toolType,
                action,
                target,
                isRejected,
                summary: isRejected
                  ? 'User rejected'
                  : isRunning
                    ? 'Executing...'
                    : isAutoExecuteWorkflowTool(name)
                      ? 'Executing...'
                      : 'Awaiting approval'
              }
            })
            .filter(call => {
              if (toolMessageIds.has(call.id)) return false
              const state = toolStates.get(call.id)
              if (!state) return true
              return state.isRejected
            })
        }

        return {
          ...message,
          displayId,
          toolDisplay,
          subAgentCard: buildSubAgentCard(message),
          pendingToolCalls,
          isRejected,
          isApproved
        }
      })
      .filter(m => {
        if (m.metadata?.ui_visibility === 'hide') return false
        // Standard visibility logic
        if (m.role === 'tool') {
          const name = m.metadata?.tool_call?.name || m.metadata?.tool_call?.function?.name || ''
          if (name === 'answer_user' || name === 'complete_workflow_with_summary') return false
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

  // Helper to remove <SYSTEM_REMINDER>...</SYSTEM_REMINDER> tags from content
  const removeSystemReminder = content => {
    if (!content) return ''
    // Handle multiline content and multiple tags
    return content.replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '').trim()
  }

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

  // Format tool title with icon, tool type class, and display text
  const formatToolTitle = (name, args) => {
    const toolFormatters = {
      read_file: args => {
        const path = args.file_path || args.path || ''
        const limit = args.limit
        const offset = args.offset
        let suffix = ''
        if (limit !== undefined && offset !== undefined) {
          suffix = ` L${offset + 1}-${offset + limit}`
        } else if (limit !== undefined) {
          suffix = ` L1-${limit}`
        } else if (offset !== undefined) {
          suffix = ` from L${offset + 1}`
        }
        return {
          icon: resolveWorkflowToolIcon(name, 'file'),
          toolType: 'tool-file',
          action: 'Read',
          target: `${path}${suffix}`
        }
      },

      write_file: args => {
        const path = args.file_path || args.path || ''
        return {
          icon: resolveWorkflowToolIcon(name, 'file'),
          toolType: 'tool-file',
          action: 'Write',
          target: path
        }
      },

      edit_file: args => {
        const path = args.file_path || args.path || ''
        return {
          icon: resolveWorkflowToolIcon(name, 'edit'),
          toolType: 'tool-file',
          action: `Edit ${path}`,
          target: ''
        }
      },

      list_dir: args => {
        const path = args.path || args.dir || '.'
        return {
          icon: resolveWorkflowToolIcon(name, 'folder'),
          toolType: 'tool-file',
          action: 'List',
          target: path
        }
      },

      glob: args => {
        const pattern = args.pattern || args.glob || ''
        return {
          icon: resolveWorkflowToolIcon(name, 'search'),
          toolType: 'tool-file',
          action: `Glob ${pattern}`,
          target: ''
        }
      },

      grep: args => {
        const pattern = args.pattern || args.query || ''
        const path = args.path || ''
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
        const cmd = args.command || ''
        return {
          icon: resolveWorkflowToolIcon(name, 'terminal'),
          toolType: 'tool-system',
          action: `Bash: ${truncateText(cmd, 60)}`,
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
    } else if (meta.title && meta.title.trim()) {
      finalAction = removeSystemReminder(meta.title)
      finalTarget = '' // Target is usually embedded in the title
    }

    // Fallback for missing action (prevents empty titles)
    if (!finalAction && !name) {
      // If it's a tool result but we lost the name, use a generic "Result"
      finalAction = t('chat.toolResult') || 'Result'
    }

    return {
      title: finalAction + (finalTarget ? ` ${finalTarget}` : ''),
      summary:
        name === 'complete_workflow_with_summary'
          ? ''
          : removeSystemReminder(meta.summary || (isError ? 'Failed' : 'Executing...')),
      isError: isError,
      displayType: meta.display_type || 'text',
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
