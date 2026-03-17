import { ref, computed } from 'vue'
import { useWorkflowStore } from '@/stores/workflow'
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
    return enhancedMessages.value.filter((m) => m.role === 'assistant').pop()
  })

  // Enhanced messages with pre-calculated display info
  const enhancedMessages = computed(() => {
    if (!workflowStore.messages || workflowStore.messages.length === 0) return []

    const rawMsgs = workflowStore.messages
    const toolStates = new Map() // tool_call_id -> { isFinal: bool, isRejected: bool, hasError: bool }
    const toolHasWaitingMsg = new Set() // tool_call_id that has an 'Awaiting' message

    // --- PASS 1: Single scan to collect all states (O(N)) ---
    const processedMsgs = rawMsgs.map((m) => {
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
        const summary = (meta.summary || '').toLowerCase()

        // Use approval_status as the primary indicator
        if (approvalStatus === 'pending') {
          toolHasWaitingMsg.add(id)
        } else if (approvalStatus === 'rejected' || approvalStatus === 'approved') {
          // Final states
          const isRejected = approvalStatus === 'rejected'
          const isError = m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected, hasError: isError })
        } else if (m.role === 'tool') {
          // Fallback: normal tool execution result (no approval flow)
          const isError = m.isError || m.is_error || meta.is_error || false
          toolStates.set(id, { isFinal: true, isRejected: false, hasError: isError })
        }
      }
      return { ...m, metadata: meta } // Cache parsed meta for Pass 2
    })

    // --- PASS 2: Filter and Transform (O(N)) ---
    return processedMsgs
      .filter((m) => {
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
        if (approvalStatus === 'rejected') {
          isRejected = true
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
            .map((call) => {
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
              return { id: call.id, icon, toolType, action, target }
            })
            .filter((call) => !toolStates.has(call.id) || !toolStates.get(call.id).isFinal)
        }

        return {
          ...message,
          displayId,
          toolDisplay,
          pendingToolCalls,
          isRejected,
          isApproved
        }
      })
      .filter((m) => {
        // Standard visibility logic
        if (m.role === 'tool') {
          const name =
            m.metadata?.tool_call?.name || m.metadata?.tool_call?.function?.name || ''
          if (name === 'answer_user' || name === 'finish_task') return false
          return true
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

  const toggleMessageExpand = (id) => {
    if (expandedMessages.value.has(id)) {
      expandedMessages.value.delete(id)
    } else {
      expandedMessages.value.add(id)
    }
  }

  const isMessageExpanded = (message) => {
    // Only force expansion for 'Ask User' to ensure visibility of interaction points.
    // Everything else (especially heavy Diffs) should be collapsed by default.
    if (message.toolDisplay?.action === 'Ask User') return true
    return expandedMessages.value.has(message.displayId)
  }

  const toggleReasoningExpand = (id) => {
    if (expandedReasonings.value.has(id)) {
      expandedReasonings.value.delete(id)
    } else {
      expandedReasonings.value.add(id)
    }
  }

  const isReasoningExpanded = (id) => expandedReasonings.value.has(id)

  // Helper to remove <SYSTEM_REMINDER>...</SYSTEM_REMINDER> tags from content
  const removeSystemReminder = (content) => {
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
      read_file: (args) => {
        const path = args.file_path || args.path || ''
        const limit = args.limit
        const offset = args.offset
        let suffix = ''
        if (limit !== undefined && offset !== undefined) {
          suffix = ` L${limit}-${offset}`
        } else if (limit !== undefined) {
          suffix = ` L${limit}`
        } else if (offset !== undefined) {
          suffix = ` @${offset}`
        }
        return { icon: 'file', toolType: 'tool-file', action: 'Read', target: `${path}${suffix}` }
      },

      write_file: (args) => {
        const path = args.file_path || args.path || ''
        return { icon: 'file', toolType: 'tool-file', action: 'Write', target: path }
      },

      edit_file: (args) => {
        const path = args.file_path || args.path || ''
        return { icon: 'edit', toolType: 'tool-file', action: `Edit ${path}`, target: '' }
      },

      list_dir: (args) => {
        const path = args.path || args.dir || '.'
        return { icon: 'folder', toolType: 'tool-file', action: 'List', target: path }
      },

      glob: (args) => {
        const pattern = args.pattern || args.glob || ''
        return { icon: 'search', toolType: 'tool-file', action: `Glob ${pattern}`, target: '' }
      },

      grep: (args) => {
        const pattern = args.pattern || args.query || ''
        const path = args.path || ''
        const action = path ? `Grep "${pattern}" in ${path}` : `Grep "${pattern}"`
        return { icon: 'search', toolType: 'tool-file', action, target: '' }
      },

      web_fetch: (args) => {
        const url = args.url || ''
        return { icon: 'link', toolType: 'tool-network', action: `Fetch ${url}`, target: '' }
      },

      web_search: (args) => {
        const query = args.query || ''
        const numResults = args.num_results
        const action =
          numResults !== undefined ? `Search "${query}" (Count: ${numResults})` : `Search "${query}"`
        return { icon: 'search', toolType: 'tool-network', action, target: '' }
      },

      bash: (args) => {
        const cmd = args.command || ''
        return {
          icon: 'terminal',
          toolType: 'tool-system',
          action: `Bash: ${truncateText(cmd, 60)}`,
          target: ''
        }
      },

      todo_create: (args) => {
        // Handle single todo creation
        const subject = args.subject || args.title || ''
        if (subject) {
          return {
            icon: 'add',
            toolType: 'tool-todo',
            action: t('workflow.todo.create'),
            target: truncateText(subject, 25)
          }
        }
        // Handle batch creation
        const tasks = args.tasks
        if (tasks && Array.isArray(tasks)) {
          return {
            icon: 'add',
            toolType: 'tool-todo',
            action: t('workflow.todo.createBatch'),
            target: `${tasks.length}项`
          }
        }
        return { icon: 'add', toolType: 'tool-todo', action: t('workflow.todo.create'), target: '' }
      },

      todo_update: (args) => {
        const subject = args.subject || args.title || ''
        const status = args.status || ''
        let statusText = ''
        if (status === 'completed') statusText = t('workflow.todo.statusCompleted')
        else if (status === 'in_progress') statusText = t('workflow.todo.statusInProgress')
        else if (status === 'pending') statusText = t('workflow.todo.statusPending')
        else statusText = status

        if (subject && statusText) {
          return {
            icon: 'check',
            toolType: 'tool-todo',
            action: `Update ${truncateText(subject, 20)} to ${statusText}`,
            target: ''
          }
        }
        return { icon: 'check', toolType: 'tool-todo', action: t('workflow.todo.update'), target: '' }
      },
      todo_list: () => ({
        icon: 'list',
        toolType: 'tool-todo',
        action: t('workflow.todo.list'),
        target: ''
      }),
      todo_get: () => ({
        icon: 'list',
        toolType: 'tool-todo',
        action: t('workflow.todo.view'),
        target: ''
      }),
      finish_task: () => ({
        icon: 'check-circle',
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
    const defaultName = name.replace(/_/g, ' ').replace(/\b\w/g, (l) => l.toUpperCase())
    return { icon: 'tool', toolType: 'tool-system', action: defaultName, target: '' }
  }

  // Standardize tool display info from metadata
  const getToolDisplayInfo = (message) => {
    const meta = message.metadata || {}
    const isError = message.isError || message.is_error || meta.is_error || false

    // 1. Try to extract tool call info
    const toolCall = meta.tool_call || {}
    const func = toolCall.function || toolCall
    const name = func.name || ''
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

    if (meta.title && meta.title.trim()) {
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
      summary: removeSystemReminder(meta.summary || (isError ? 'Failed' : 'Executing...')),
      isError: isError,
      displayType: meta.display_type || 'text',
      icon: finalIcon,
      toolType: finalToolType,
      action: finalAction,
      target: finalTarget
    }
  }

  // Get diff markdown for file edits
  const getDiffMarkdown = (content) => {
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
      const newStr =
        data.new_string !== undefined ? data.new_string : data.content || ''
      const filePath = data.file_path || data.path || 'file'

      // If it's just raw content without diff semantics, return as code block
      if (data.old_string === undefined && data.new_string === undefined && !data.content) {
        return typeof content === 'string' ? content : JSON.stringify(content, null, 2)
      }

      // Generate standard unidiff-like format for better highlighting
      let diffContent = `File: **${filePath}**\n\n\`\`\`diff\n`

      const UI_LINE_LIMIT = 500 // Limit lines shown in UI for performance

      if (data.old_string !== undefined) {
        // Use diff library to generate proper line-by-line diff
        const changes = Diff.diffLines(oldStr, newStr)
        let lineCount = 0

        changes.forEach((change) => {
          if (lineCount >= UI_LINE_LIMIT) return

          const lines = change.value.split('\n')
          // Remove last empty line if exists
          if (lines[lines.length - 1] === '') {
            lines.pop()
          }

          lines.forEach((line) => {
            if (lineCount >= UI_LINE_LIMIT) return

            if (change.added) {
              diffContent += `+ ${line}\n`
              lineCount++
            } else if (change.removed) {
              diffContent += `- ${line}\n`
              lineCount++
            } else {
              // Unchanged lines - show with space prefix for context (optional, can be omitted for compactness)
              // diffContent += `  ${line}\n`
              // lineCount++
              // For compactness, we skip unchanged lines like ApprovalDialog does
            }
          })
        })

        if (lineCount >= UI_LINE_LIMIT) {
          diffContent += `... (truncated for preview)\n`
        }
      } else {
        // For new files or overwrites: "- " (empty line) then "+ content"
        diffContent += `- \n`
        const newLines = newStr.split('\n')
        const displayLines = newLines.slice(0, UI_LINE_LIMIT)

        displayLines.forEach((line) => (diffContent += `+ ${line}\n`))
        if (newLines.length > UI_LINE_LIMIT) {
          diffContent += `+ ... (${newLines.length - UI_LINE_LIMIT} lines truncated)\n`
        }
      }

      diffContent += '```'
      return diffContent
    } catch (e) {
      return typeof content === 'string' ? content : JSON.stringify(content)
    }
  }

  // Parse choice content for Ask User tool
  const parseChoiceContent = (content) => {
    try {
      return JSON.parse(content)
    } catch (e) {
      return { question: content, options: [] }
    }
  }

  // Helper to parse message content
  const getParsedMessage = (message) => {
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
        parsedToolCalls = parsedToolCalls.filter((call) => {
          const name = call?.function?.name || call?.name
          return name !== 'finish_task' && name !== 'answer_user'
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
    getParsedMessage
  }
}
