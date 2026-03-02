<template>
  <div class="workflow-layout">
    <titlebar>
      <template #left>
        <el-tooltip :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)" placement="right"
          :hide-after="0" :enterable="false">
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
        </el-tooltip>
      </template>
      <template #center> </template>
      <template #right>
        <div class="icon-btn upperLayer pin-btn" @click="onPin" :class="{ active: isAlwaysOnTop }">
          <el-tooltip :content="$t(`common.${isAlwaysOnTop ? 'unpin' : 'pin'}`)" :hide-after="0" :enterable="false"
            placement="bottom">
            <cs name="pin" />
          </el-tooltip>
        </div>
      </template>
    </titlebar>

    <div class="workflow-main">
      <el-aside :width="sidebarWidth" :class="{ collapsed: sidebarCollapsed }" class="sidebar">
        <div class="sidebar-header upperLayer">
          <el-input v-model="searchQuery" :placeholder="$t('chat.searchChat')" :clearable="true" round>
            <template #prefix>
              <cs name="search" />
            </template>
          </el-input>
        </div>
        <div v-show="!sidebarCollapsed" class="workflow-list">
          <div class="list">
            <div class="item" v-for="wf in filteredWorkflows" :key="wf.id" @click="selectWorkflow(wf.id)"
              @mouseenter="hoveredWorkflowIndex = wf.id" @mouseleave="hoveredWorkflowIndex = null" :class="{
                active: wf.id === currentWorkflowId,
                disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId
              }">
              <div class="workflow-title">{{ wf.title || wf.userQuery }}</div>
              <div class="workflow-status" v-if="wf.status">
                <span :class="['status-indicator', wf.status.toLowerCase()]"></span>
                {{ wf.status }}
              </div>
              <div class="icons" v-show="wf.id === hoveredWorkflowIndex">
                <div class="icon icon-edit" @click.stop="onEditWorkflow(wf.id)">
                  <cs name="edit" />
                </div>
                <div class="icon icon-delete" @click.stop="onDeleteWorkflow(wf.id)">
                  <cs name="delete" />
                </div>
              </div>
            </div>
          </div>
        </div>
      </el-aside>

      <!-- main container -->
      <el-container class="main-container">
        <div class="messages" ref="messagesRef">
          <div v-for="(message, index) in enhancedMessages" :key="message.id" class="message"
            :class="[message.role, message.stepType?.toLowerCase()]">
            <div class="avatar" v-if="message.role === 'user'">
              <cs name="talk" class="user-icon" />
            </div>
            <div class="content-container">
              <div class="content" v-if="message.role === 'user'">
                <div class="msg-ops" v-if="index > 0">
                  <el-tooltip :content="$t('common.delete')" placement="top">
                    <span class="op-icon" @click="onDeleteMessage(message.id)">
                      <cs name="trash" size="12px" />
                    </span>
                  </el-tooltip>
                </div>
                <pre class="simple-text">{{ message.message }}</pre>
              </div>
              <div v-else class="ai-content">
                <!-- CLI Style Tool Call (Results) -->
                <div v-if="message.role === 'tool'" class="cli-tool-call expandable"
                  :class="{ error: message.toolDisplay.isError }" @click="toggleMessageExpand(message.id)">
                  <div class="tool-line title-wrap">
                    <span class="status-dot" :class="{ 'error': message.toolDisplay.isError }">●</span>
                    <span class="tool-title">{{ message.toolDisplay.title }}</span>
                  </div>
                  <div class="tool-line summary">
                    <span class="corner-icon">⎿</span>
                    <span class="summary-text">{{ message.toolDisplay.summary }}</span>
                    <span class="expand-hint" v-if="!isMessageExpanded(message.id)">(click to expand)</span>
                  </div>
                  <div v-if="isMessageExpanded(message.id)" class="tool-detail">
                    <markdown v-if="message.toolDisplay.displayType === 'diff'" :content="message.message" />
                    <pre v-else class="raw-content">{{ message.message }}</pre>
                  </div>
                </div>

                <!-- Regular Assistant Content -->
                <div v-else>
                  <!-- Thought/Content FIRST (Separate reasoning field has priority) -->
                  <div class="thought-content" v-if="message.reasoning || message.stepType === 'Think'">
                    {{ message.reasoning || message.message }}
                  </div>
                  <markdown v-if="getParsedMessage(message).content" :content="getParsedMessage(message).content" />

                  <!-- Tool Call Indicators SECOND (Only pending ones) -->

                  <div v-if="message.pendingToolCalls?.length > 0" class="cli-tool-calls-container">
                    <div v-for="call in message.pendingToolCalls" :key="call.id" class="cli-tool-call pending">
                      <div class="tool-line title-wrap">
                        <span class="tool-title">
                          <cs name="loading" class="status-icon rotating" /> {{ call.title }}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>

                <!-- Original Ops -->
                <div class="msg-ops-container">
                  <div class="msg-ops floating" v-if="index > 0">
                    <el-tooltip :content="$t('common.delete')" placement="top">
                      <span class="op-icon" @click="onDeleteMessage(message.id)">
                        <cs name="trash" size="12px" />
                      </span>
                    </el-tooltip>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <!-- Active Chatting State -->
          <div v-if="isChatting && chatState.content" class="message assistant chatting">
            <div class="content-container">
              <div class="ai-content">
                <markdown :content="chatState.content" />
              </div>
            </div>
          </div>
        </div>

        <div class="todo-list-wrapper" v-if="todoList.length > 0">
          <TodoList :items="todoList" />
        </div>

        <!-- footer -->
        <el-footer class="input-container">
          <!-- Slash Command Suggestion Panel -->
          <div v-if="showSkillSuggestions && filteredSystemSkills.length > 0" class="slash-command-panel">
            <div v-for="(skill, idx) in filteredSystemSkills" :key="skill.name" class="command-item"
              :class="{ active: idx === selectedSkillIndex }" @click="onSkillSelect(skill)">
              <div class="command-name">/{{ skill.name }}</div>
              <div class="command-desc">{{ skill.description }}</div>
            </div>
          </div>

          <div class="input">
            <el-input ref="inputRef" v-model="inputMessage" type="textarea" :autosize="{ minRows: 1, maxRows: 10 }"
              :placeholder="$t('chat.inputMessagePlaceholder', { at: '/' })" @keydown="onInputKeyDown"
              @compositionstart="onCompositionStart" @compositionend="onCompositionEnd" />

            <div class="input-footer">
              <div class="footer-left">
                <div class="agent-selector-wrap" :class="{ disabled: currentWorkflowId }">
                  <AgentSelector v-model="selectedAgent" :agent="currentWorkflow?.agentId
                    ? agentStore.agents.find(a => a.id === currentWorkflow.agentId)
                    : null
                    " :disabled="!!currentWorkflowId" />
                </div>

                <!-- Authorized Paths -->
                <div v-if="currentWorkflowId" class="allowed-paths-wrap">
                  <el-popover placement="top" :width="300" trigger="click" popper-class="paths-popover">
                    <template #reference>
                      <div class="paths-summary upperLayer" :class="{ empty: allowedPaths.length === 0 }">
                        <cs name="folder" size="14px" />
                        <span class="path-text">{{ displayAllowedPath || $t('settings.agent.workingDirectory') }}</span>
                        <span v-if="allowedPaths && allowedPaths.length > 1" class="path-count">+{{ allowedPaths.length
                          - 1 }}</span>
                      </div>
                    </template>
                    <div class="paths-detail">
                      <div class="paths-header">
                        <span>{{ $t('settings.agent.authorizedPaths') }}</span>
                        <el-button size="small" type="primary" link @click="onAddPath">
                          <cs name="add" size="14px" />
                        </el-button>
                      </div>
                      <div class="paths-list">
                        <div v-for="(path, idx) in allowedPaths" :key="idx" class="path-item">
                          <span class="path-name" :title="path">{{ path }}</span>
                          <div class="path-ops">
                            <cs name="trash" size="12px" @click="onRemovePath(idx)" />
                          </div>
                        </div>
                        <div v-if="allowedPaths.length === 0" class="empty-paths">
                          {{ $t('settings.agent.authorizedPathsTip') }}
                        </div>
                      </div>
                    </div>
                  </el-popover>
                </div>

                <div class="icons">
                  <el-tooltip :content="$t('workflow.autoApproveTooltip')" placement="top">
                    <label class="icon-btn upperLayer" :class="{ active: autoApproveTools }">
                      <cs name="tool" class="small" />
                    </label>
                  </el-tooltip>
                  <el-tooltip :content="$t('workflow.newWorkflow')" :hide-after="0" :enterable="false" placement="top">
                    <label @click="createNewWorkflow" :class="{ disabled: isRunning }">
                      <cs name="new-chat" class="small" :class="{ disabled: isRunning }" />
                    </label>
                  </el-tooltip>
                </div>
              </div>
              <div class="icons">
                <el-button
                  v-if="!isRunning && currentWorkflowId && currentWorkflow?.status !== 'completed' && currentWorkflow?.status !== 'error'"
                  size="small" round type="primary" @click="onContinue">
                  {{ $t('workflow.continue') }}
                </el-button>
                <cs name="stop" @click="onStop" v-if="isRunning" />
                <cs name="send" @click="onSendMessage" :class="{ disabled: !canSendMessage }" />
              </div>
            </div>
          </div>
        </el-footer>
      </el-container>
    </div>

    <!-- edit workflow dialog -->
    <el-dialog v-model="editWorkflowDialogVisible" :title="$t('workflow.editWorkflowTitle')"
      :close-on-press-escape="false" width="50%">
      <el-form>
        <el-form-item :label="$t('workflow.workflowTitle')">
          <el-input v-model="editWorkflowTitle" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editWorkflowDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="onSaveEditWorkflow">{{ $t('common.save') }}</el-button>
      </template>
    </el-dialog>

    <ApprovalDialog v-model="approvalVisible" :action="approvalAction" :details="approvalDetails"
      :loading="approvalLoading" @approve="onApproveAction" @approveAll="onApproveAllAction" @reject="onRejectAction" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'

import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'

import Titlebar from '@/components/window/Titlebar.vue'
import Markdown from '@/components/chat/Markdown.vue'
import AgentSelector from '@/components/workflow/AgentSelector.vue'
import TodoList from '@/components/workflow/TodoList.vue'
import ApprovalDialog from '@/components/workflow/ApprovalDialog.vue'

// Import types
import { getTodoListForWorkflow } from '@/pkg/workflow/tools/todoList'
import { MarkdownStreamParser } from '@/libs/markdown-stream-parser'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

const unlistenFocusInput = ref(null)
const unlistenWorkflowEvents = ref(null)
const osType = ref('') // To store OS type from backend
const hoveredWorkflowIndex = ref(null) // For workflow hover effects

// approval dialog
const approvalVisible = ref(false)
const approvalAction = ref('')
const approvalDetails = ref('')
const approvalRequestId = ref('')
const approvalLoading = ref(false)

// Chatting state for real-time streaming
const chattingParser = new MarkdownStreamParser()
const isChatting = computed(() => workflowStore.isRunning)
const chatState = ref({
  content: '',
  blocks: []
})

// edit workflow dialog
const editWorkflowDialogVisible = ref(false)
const editWorkflowId = ref(null)
const editWorkflowTitle = ref('')

const sidebarCollapsed = ref(!windowStore.workflowSidebarShow)
const sidebarWidth = computed(() => (sidebarCollapsed.value ? '0px' : '200px'))
const searchQuery = ref('')
const inputMessage = ref('')
const selectedAgent = ref(null)
const autoApproveTools = ref(true)
const composing = ref(false)
const compositionJustEnded = ref(false)
const messagesRef = ref(null)
const inputRef = ref(null)

// System Skills (from ~/.chatspeed/skills etc) slash command logic
const systemSkills = ref([])
const showSkillSuggestions = ref(false)
const selectedSkillIndex = ref(0)
const filteredSystemSkills = computed(() => {
  // Only search if starts with /
  if (!inputMessage.value.startsWith('/')) return []
  const query = inputMessage.value.substring(1).toLowerCase()
  return systemSkills.value.filter(skill =>
    skill.name.toLowerCase().includes(query) ||
    (skill.description && skill.description.toLowerCase().includes(query))
  )
})

const fetchSystemSkills = async () => {
  try {
    const result = await invokeWrapper('get_system_skills')
    systemSkills.value = result || []
  } catch (error) {
    console.error('Failed to fetch system skills:', error)
  }
}
const onSkillSelect = (skill) => {
  // Replace the slash command with the full skill command
  inputMessage.value = '/' + skill.name + ' '
  showSkillSuggestions.value = false
  selectedSkillIndex.value = 0
  nextTick(() => {
    if (inputRef.value) {
      inputRef.value.focus()
    }
  })
}
watch(inputMessage, (newVal) => {
  // TRIGGERS ONLY if '/' is the very first character of the whole input
  if (newVal === '/') {
    showSkillSuggestions.value = systemSkills.value.length > 0
    selectedSkillIndex.value = 0
  } else if (!newVal.startsWith('/') || newVal === '') {
    showSkillSuggestions.value = false
  }
})

watch(filteredSystemSkills, () => {
  selectedSkillIndex.value = 0
})

// Authorized paths management
const allowedPaths = computed(() => {
  const paths = currentWorkflow.value?.allowedPaths
  console.log('currentWorkflow:', currentWorkflow.value)
  if (!paths) return []
  try {
    const parsed = typeof paths === 'string' ? JSON.parse(paths) : paths
    console.log('Workflow.vue: parsed allowedPaths:', parsed)
    return parsed
  } catch (e) {
    return []
  }
})

const displayAllowedPath = computed(() => {
  const paths = allowedPaths.value
  console.log('Workflow.vue: computing displayAllowedPath for:', paths)
  if (!paths || paths.length === 0) return t('settings.agent.workingDirectory')
  const firstPath = paths[0]
  if (!firstPath) return t('settings.agent.workingDirectory')
  // Try to get last segment of path
  const parts = firstPath.split(/[/\\]/).filter(p => p !== '')
  const result = parts[parts.length - 1] || firstPath
  console.log('Workflow.vue: displayAllowedPath result:', result)
  return result
})

const onAddPath = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.agent.selectDirectory')
    })
    if (selected) {
      const newPaths = [...allowedPaths.value]
      if (!newPaths.includes(selected)) {
        newPaths.push(selected)
        await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
        // Immediately notify executor to update path_guard in memory
        await invokeWrapper('workflow_signal', {
          sessionId: currentWorkflowId.value,
          signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
        })
      }
    }
  } catch (error) {
    console.error('Failed to add path:', error)
  }
}

const onRemovePath = async (index) => {
  const newPaths = [...allowedPaths.value]
  newPaths.splice(index, 1)
  await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
  // Immediately notify executor
  await invokeWrapper('workflow_signal', {
    sessionId: currentWorkflowId.value,
    signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
  })
}

// Message expansion state
const expandedMessages = ref(new Set())
const toggleMessageExpand = (id) => {
  if (expandedMessages.value.has(id)) {
    expandedMessages.value.delete(id)
  } else {
    expandedMessages.value.add(id)
  }
}
const isMessageExpanded = (id) => expandedMessages.value.has(id)

// Mirroring the backend's title generation logic in JS
const formatToolTitle = (name, args) => {
  if (!name) return 'Tool'
  const displayNames = {
    'read_file': 'Read',
    'write_file': 'Write',
    'edit_file': 'Edit',
    'list_dir': 'List',
    'grep': 'Grep',
    'grep_search': 'Grep',
    'web_search': 'Search',
    'web_fetch': 'Fetch',
    'bash': 'Bash',
    'todo_create': 'TodoCreate',
    'todo_update': 'TodoUpdate',
    'todo_list': 'TodoList',
    'todo_get': 'TodoGet'
  }

  const toolName = displayNames[name] || name.replace('todo_', 'Todo')

  let parsedArgs = args
  if (typeof args === 'string' && args.trim().startsWith('{')) {
    try { parsedArgs = JSON.parse(args) } catch (e) { /* keep as string */ }
  }

  if (!parsedArgs || typeof parsedArgs !== 'object') {
    return toolName + (parsedArgs ? `(${parsedArgs})` : '')
  }

  const parts = Object.entries(parsedArgs).map(([k, v]) => {
    let val = v
    if (typeof v === 'string') {
      val = v.length > 40 ? `"${v.substring(0, 37)}..."` : `"${v}"`
    } else {
      val = JSON.stringify(v)
    }
    return `${k}: ${val}`
  })

  return parts.length > 0 ? `${toolName}(${parts.join(', ')})` : toolName
}

// Standardize tool display info from metadata
const getToolDisplayInfo = (message) => {
  const meta = message.metadata || {}

  // Prioritize top-level error flag from message object
  const isError = message.isError || message.is_error || meta.is_error || false

  // If backend provided pre-formatted title/summary, use them (Tool Results)
  if (meta.title && meta.summary) {
    return {
      title: meta.title,
      summary: meta.summary,
      isError: isError,
      displayType: meta.display_type || 'text'
    }
  }

  // Fallback or for Pending calls (reconstruct from tool_call metadata)
  const toolCall = meta.tool_call || {}
  const func = toolCall.function || toolCall
  const name = func.name || ''
  const args = func.arguments || func.input || {}

  return {
    title: formatToolTitle(name, args),
    summary: 'Executing...',
    isError: isError,
    displayType: 'text'
  }
}

const onCompositionStart = () => {
  composing.value = true
}

const onCompositionEnd = () => {
  composing.value = false
  compositionJustEnded.value = true
  setTimeout(() => {
    compositionJustEnded.value = false
  }, 100)
}

const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)

const workflows = computed(() => workflowStore.workflows)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const messages = computed(() => workflowStore.messages)
const isRunning = computed(() => workflowStore.isRunning)
const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)

// Enhanced messages with pre-calculated display info
const enhancedMessages = computed(() => {
  const msgs = messages.value

  // Calculate completed tool IDs once for the entire list
  const completedIds = new Set(
    msgs
      .filter(m => m.role === 'tool' && m.metadata?.tool_call_id)
      .map(m => m.metadata.tool_call_id)
  )

  return msgs.map(message => {
    // Pre-calculate parsed content to avoid multiple calls in filter and template
    const parsed = getParsedMessage(message)
    const toolDisplay = getToolDisplayInfo(message)

    // Pre-calculate pending tool calls
    let pendingToolCalls = []
    const toolCalls = message.metadata?.tool_calls || []
    if (toolCalls.length > 0) {
      pendingToolCalls = toolCalls
        .filter(call => !completedIds.has(call.id))
        .map(call => ({
          id: call.id,
          title: formatToolTitle(call.function?.name || call.name, call.function?.arguments || call.arguments)
        }))
    }

    return {
      ...message,
      parsed,
      toolDisplay,
      pendingToolCalls
    }
  }).filter(m => {
    // 1. Visibility logic for tool results (Observations)
    if (m.role === 'tool') {
      const meta = m.metadata || {}
      const toolCall = meta.tool_call || {}
      const name = toolCall.name || (toolCall.function && toolCall.function.name) || ''

      // Hide internal orchestration tools
      if (name === 'answer_user' || name === 'finish_task') return false

      // Keep everything else (including todo_* results, which provide feedback)
      return true
    }

    // 2. Visibility logic for Assistant messages
    if (m.role === 'assistant') {
      // Show if there is any text content (message, parsed content, or reasoning)
      const hasTextContent = (m.message && m.message.trim()) || 
                            (m.parsed && m.parsed.content && m.parsed.content.trim()) || 
                            (m.reasoning && m.reasoning.trim())

      if (hasTextContent) return true

      // Show if there are pending tool calls (even if no text)
      if (m.pendingToolCalls && m.pendingToolCalls.length > 0) return true

      // Hide empty assistant turns (often used just for thinking/routing)
      return false
    }

    // 3. User messages always shown
    return true
  })

})
// Get todo list from the store
const todoList = computed(() => workflowStore.todoList)

const filteredWorkflows = computed(() => {
  if (!searchQuery.value) return workflows.value
  return workflows.value.filter(wf =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.value.toLowerCase())
  )
})

const canSendMessage = computed(
  () => inputMessage.value.trim() !== '' && selectedAgent.value
)

// Watch for state changes to handle UI side effects
watch(() => currentWorkflow.value?.status, (newStatus) => {
  // If state is no longer Paused, we should hide any open approval dialog
  if (newStatus !== 'paused' && approvalVisible.value) {
    approvalVisible.value = false
  }
})

const canSwitchWorkflow = computed(() => {
  // Can't switch if a workflow is currently running
  return !isRunning.value
})

// Watch for workflow changes to update UI
watch(currentWorkflow, newWorkflow => {
  if (newWorkflow) {
    // Scroll to bottom when new workflow is selected
    nextTick(() => {
      scrollToBottom()
    })
  }
})

// Watch for messages to scroll to bottom
watch(
  messages,
  () => {
    nextTick(() => {
      scrollToBottom()
    })
  },
  { deep: true }
)

// Helper to parse message content (handles raw JSON from ReAct Think steps)
const getParsedMessage = (message) => {
  let content = message.message || ''
  let toolCalls = []
  let isError = false

  if (message.role === 'tool') {
    if (content.toLowerCase().startsWith('error:') || content.toLowerCase().includes('failed')) {
      isError = true
    }
    // For tool messages, we show the call info if available in metadata
    if (message.metadata?.tool_call) {
      toolCalls = [message.metadata.tool_call]
    } else if (message.metadata?.toolCalls) {
      toolCalls = message.metadata.toolCalls
    }
  } else if (message.role === 'assistant') {
    // For assistant messages, we only show tool calls if it's NOT a Think step
    // or if the user wants to see them anyway (but following the request to hide unexecuted ones)
    if (message.stepType !== 'Think') {
      toolCalls = message.metadata?.toolCalls || message.metadata?.tool_calls || []
    }
  }

  try {
    // Check if it's a JSON response
    const trimmed = content.trim()
    if (trimmed.startsWith('{')) {
      const parsed = JSON.parse(trimmed)
      let parsedContent = ''
      let parsedToolCalls = []

      // 1. Handle standard content field (OpenAI style)
      if (parsed.content) {
        parsedContent = parsed.content
      }

      // 2. Handle ReAct style tool calls {"tool": {"name": "...", "arguments": {...}}}
      if (parsed.tool) {
        const name = parsed.tool.name
        const args = parsed.tool.arguments || {}

        if (name === 'answer_user') {
          parsedContent = args.text || ''
        } else if (name === 'finish_task') {
          parsedContent = args.summary || ''
        }
      }

      // 3. Handle OpenAI style tool_calls array
      parsedToolCalls = parsed.tool_calls || parsed.toolCall || []

      // If assistant Think step, we still might want to hide these if they are "unexecuted"
      if (message.role === 'assistant' && message.stepType === 'Think') {
        parsedToolCalls = []
      }

      return {
        content: parsedContent,
        toolCalls: toolCalls.length > 0 ? toolCalls : parsedToolCalls,
        isError
      }
    }
  } catch (e) {
    // Not JSON, fall back to raw message
  }

  return {
    content,
    toolCalls,
    isError
  }
}

const scrollToBottom = () => {
  if (messagesRef.value) {
    const el = messagesRef.value
    // Check if user is near bottom (with 100px threshold)
    const isAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100

    if (isAtBottom) {
      nextTick(() => {
        el.scrollTop = el.scrollHeight
      })
    }
  }
}

onMounted(async () => {
  unlistenFocusInput.value = await listen('cs://workflow-focus-input', event => {
    if (event.payload && event.payload.windowLabel === settingStore.windowLabel) {
      if (inputRef.value) {
        inputRef.value.focus()
      }
    }
  })

  try {
    const osInfo = await invokeWrapper('get_os_info')
    osType.value = osInfo.os
  } catch (error) {
    console.error('Failed to get OS info:', error)
  }

  await workflowStore.loadWorkflows()
  await agentStore.fetchAgents()
  await fetchSystemSkills()

  if (agentStore.agents.length > 0) {
    selectedAgent.value = agentStore.agents[0]
  }

  // Load the last workflow if available
  if (workflowStore.workflows.length > 0) {
    await selectWorkflow(workflowStore.workflows[0].id)
  }

  windowStore.initWorkflowWindowAlwaysOnTop()
  window.addEventListener('keydown', onGlobalKeyDown)
})

onBeforeUnmount(() => {
  if (unlistenWorkflowEvents.value) {
    unlistenWorkflowEvents.value()
  }
  unlistenFocusInput.value()
  window.removeEventListener('keydown', onGlobalKeyDown)
})

const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  windowStore.setWorkflowSidebarShow(!sidebarCollapsed.value)
}

const setupWorkflowEvents = async sessionId => {
  if (unlistenWorkflowEvents.value) {
    unlistenWorkflowEvents.value()
    unlistenWorkflowEvents.value = null
  }

  const eventName = `workflow://event/${sessionId}`
  unlistenWorkflowEvents.value = await listen(eventName, event => {
    const payload = event.payload
    console.log('Workflow Event:', payload)

    if (payload.type === 'state') {
      workflowStore.updateWorkflowStatus(sessionId, payload.state)

      // If we move out of Thinking/Executing, reset the parser
      if (payload.state !== 'thinking' && payload.state !== 'executing') {
        chattingParser.reset()
        chatState.value.content = ''
        chatState.value.blocks = []
      }
    } else if (payload.type === 'chunk') {
      // Direct text chunk from LLM or StreamParser
      chatState.value.content += payload.content
      chatState.value.blocks = chattingParser.process(payload.content)

      nextTick(() => scrollToBottom())
    } else if (payload.type === 'message') {
      // ReAct engine sends incremental messages or chunks
      workflowStore.addMessage({
        sessionId: sessionId,
        role: payload.role,
        message: payload.content,
        reasoning: payload.reasoning,
        stepType: payload.step_type,
        stepIndex: payload.step_index,
        isError: payload.is_error,
        errorType: payload.error_type,
        metadata: payload.metadata
      })

      // Message finalized, clear chatting buffer
      chattingParser.reset()
      chatState.value.content = ''
      chatState.value.blocks = []
    } else if (payload.type === 'confirm') {
      approvalRequestId.value = payload.id
      approvalAction.value = payload.action
      approvalDetails.value = payload.details
      approvalVisible.value = true
    } else if (payload.type === 'sync_todo') {
      workflowStore.setTodoList(payload.todo_list)
    }
  })
}

const selectWorkflow = async id => {
  if (!canSwitchWorkflow.value) {
    console.warn('Cannot switch workflow while another is running')
    return
  }

  // Select the workflow in store
  await workflowStore.selectWorkflow(id)

  if (workflowStore.currentWorkflow) {
    const agent = agentStore.agents.find(a => a.id === workflowStore.currentWorkflow.agentId)
    if (agent) {
      selectedAgent.value = agent
      // Setup event listeners for the existing session
      await setupWorkflowEvents(id)
    }
  }
}

const startNewWorkflow = async (prompt) => {
  if (!selectedAgent.value) {
    console.error('No agent selected')
    return
  }

  if (!prompt || !prompt.trim()) return

  try {
    console.log('Initiating workflow creation...')
    // Get allowed paths from selected agent
    let agentAllowedPaths = []
    if (selectedAgent.value.allowedPaths) {
      try {
        agentAllowedPaths = typeof selectedAgent.value.allowedPaths === 'string'
          ? JSON.parse(selectedAgent.value.allowedPaths)
          : selectedAgent.value.allowedPaths
      } catch (e) {
        console.error('Failed to parse agent allowedPaths:', e)
      }
    }

    // 1. Create workflow in DB first to get a session_id
    const res = await invokeWrapper('create_workflow', {
      workflow: {
        id: `session_${Date.now()}`,
        userQuery: prompt,
        agentId: selectedAgent.value.id,
        status: 'pending',
        allowedPaths: JSON.stringify(agentAllowedPaths),
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString()
      }
    })

    const newWorkflowId = typeof res === 'string' ? res : (res.id || res)
    console.log('Workflow session created:', newWorkflowId)

    // 2. Sync UI state
    await workflowStore.loadWorkflows()
    await workflowStore.selectWorkflow(newWorkflowId)
    await setupWorkflowEvents(newWorkflowId)

    // 4. Trigger engine
    console.log('Calling workflow_start backend command...')
    await invokeWrapper('workflow_start', {
      sessionId: newWorkflowId,
      agentId: selectedAgent.value.id,
      initialPrompt: prompt
    })
    console.log('Workflow engine started successfully')
    nextTick(() => scrollToBottom())
  } catch (error) {
    console.error('Failed to start workflow:', error)
    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
  }
}

const onApproveAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: true,
      id: approvalRequestId.value,
      tool_name: approvalAction.value,
      tool_args: {} // Should ideally be passed from backend if needed
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to approve action:', error)
  } finally {
    approvalLoading.value = false
  }
}

const onApproveAllAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: true,
      approve_all: true,
      id: approvalRequestId.value,
      tool_name: approvalAction.value,
      tool_args: {}
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to approve all actions:', error)
  } finally {
    approvalLoading.value = false
  }
}

const onRejectAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: false,
      id: approvalRequestId.value,
      tool_name: approvalAction.value
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to reject action:', error)
  } finally {
    approvalLoading.value = false
  }
}

const onDeleteMessage = async (messageId) => {
  if (!currentWorkflowId.value || !messageId) return

  try {
    // 1. Remove from local store
    await workflowStore.deleteMessage(currentWorkflowId.value, messageId)
    // 2. Refresh UI list
    await workflowStore.loadMessages(currentWorkflowId.value)
  } catch (error) {
    console.error('Failed to delete message:', error)
  }
}

const onSendMessage = async () => {
  if (!canSendMessage.value) return

  const message = inputMessage.value
  inputMessage.value = ''
  console.log('Sending message to workflow:', message)

  if (!currentWorkflowId.value) {
    // Start brand new workflow
    await startNewWorkflow(message)
  } else {
    // 1. Add to UI and DB
    await workflowStore.addMessage({
      sessionId: currentWorkflowId.value,
      role: 'user',
      message: message
    })

    nextTick(() => scrollToBottom())

    // 2. Decide: Signal or Re-start?
    if (isRunning.value) {
      // Just send signal to the running loop
      try {
        const signal = JSON.stringify({
          type: 'user_input',
          content: message
        })
        const res = await invokeWrapper('workflow_signal', {
          sessionId: currentWorkflowId.value,
          signal: signal
        })
        console.log('Signal sent successfully:', res)
      } catch (error) {
        console.error('Failed to send signal:', error)
      }
    } else {
      // Engine is stopped (Completed or Error).
      // Re-trigger workflow_start to "wake up" the Agent.
      try {
        await invokeWrapper('workflow_start', {
          sessionId: currentWorkflowId.value,
          agentId: selectedAgent.value.id,
          initialPrompt: message
        })
      } catch (error) {
        console.error('Failed to resume workflow:', error)
        showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
      }
    }
  }
}

const onInputKeyDown = event => {
  if (composing.value || compositionJustEnded.value) return

  if (showSkillSuggestions.value) {
    if (event.key === 'Enter') {
      event.preventDefault()
      if (filteredSystemSkills.value.length > 0) {
        onSkillSelect(filteredSystemSkills.value[selectedSkillIndex.value])
      } else {
        showSkillSuggestions.value = false
      }
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault() // Prevent cursor moving to start
      selectedSkillIndex.value = (selectedSkillIndex.value - 1 + filteredSystemSkills.value.length) % filteredSystemSkills.value.length
      return
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault() // Prevent cursor moving to end
      selectedSkillIndex.value = (selectedSkillIndex.value + 1) % filteredSystemSkills.value.length
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      showSkillSuggestions.value = false
      return
    }
  }

  if (event.key === 'Enter') {
    const shouldSend =
      settingStore.settings.sendMessageKey === 'Enter' ? !event.shiftKey : event.shiftKey
    if (shouldSend) {
      event.preventDefault()
      onSendMessage()
    }
  }
}

const onContinue = async () => {
  if (!currentWorkflowId.value || isRunning.value) return

  try {
    // If it's paused, we might need to send a signal,
    // but usually 'workflow_start' with no prompt works to resume the loop if it's not active.
    await invokeWrapper('workflow_start', {
      sessionId: currentWorkflowId.value,
      agentId: selectedAgent.value.id
    })
  } catch (error) {
    console.error('Failed to continue workflow:', error)
    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
  }
}

const onStop = async () => {
  if (currentWorkflowId.value) {
    try {
      await invokeWrapper('workflow_stop', {
        sessionId: currentWorkflowId.value
      })
      workflowStore.setRunning(false)
    } catch (error) {
      console.error('Failed to stop workflow:', error)
    }
  }
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
}

const onEditWorkflow = id => {
  editWorkflowId.value = id
  editWorkflowTitle.value = workflows.value.find(wf => wf.id === id)?.title || ''
  editWorkflowDialogVisible.value = true
}

const onSaveEditWorkflow = async () => {
  if (!editWorkflowId.value) return

  try {
    await invokeWrapper('update_workflow_title', {
      sessionId: editWorkflowId.value,
      title: editWorkflowTitle.value
    })

    // Reload workflows to get updated data
    await workflowStore.loadWorkflows()

    editWorkflowDialogVisible.value = false
    editWorkflowTitle.value = ''
    editWorkflowId.value = null
  } catch (error) {
    console.error('Failed to update workflow:', error)
  }
}

const onDeleteWorkflow = id => {
  ElMessageBox.confirm(t('workflow.confirmDeleteWorkflow'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel')
  }).then(async () => {
    try {
      await invokeWrapper('delete_workflow', { sessionId: id })

      // If deleting the current workflow, clear it
      if (id === currentWorkflowId.value) {
        workflowStore.clearCurrentWorkflow()
      }

      // Reload workflows
      await workflowStore.loadWorkflows()

      // Load the last workflow if available
      if (workflows.value.length > 0) {
        await selectWorkflow(workflows.value[0].id)
      }
    } catch (error) {
      console.error('Failed to delete workflow:', error)
    }
  })
}

const createNewWorkflow = () => {
  // Clear current workflow
  workflowStore.clearCurrentWorkflow()

  // Clear input and focus
  inputMessage.value = ''
  nextTick(() => {
    if (inputRef.value) {
      inputRef.value.focus()
    }
  })
}

const onGlobalKeyDown = event => {
  // Use OS type from backend. `std::env::consts::OS` returns "macos" for macOS.
  const isMac = osType.value === 'macos'
  const modifierPressed = isMac ? event.metaKey : event.ctrlKey

  if (modifierPressed) {
    switch (event.key.toLowerCase()) {
      case 'n':
        event.preventDefault()
        createNewWorkflow()
        break
      case 'b':
        event.preventDefault()
        onToggleSidebar()
        break
    }
  }
}
</script>

<style lang="scss">
.workflow-layout {
  height: 100vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;

  .workflow-main {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: row;

    .sidebar {
      border-right: 1px solid var(--cs-border-color);
      display: flex;
      flex-direction: column;
      height: 100%;
      transition: width 0.3s ease;

      .sidebar-header {
        padding: 10px;
        flex-shrink: 0;

        .el-input {
          box-sizing: border-box;

          .el-input__wrapper {
            padding: 0;
            background: var(--cs-input-bg-color) !important;
            border-radius: var(--cs-border-radius-xxl);
            font-size: var(--cs-font-size-sm);
          }

          .el-input__prefix {
            display: flex;
            align-items: center;
            padding-left: var(--cs-space-sm);

            .cs {
              font-size: var(--cs-font-size-md);
              color: var(--cs-text-color-secondary);
            }
          }
        }
      }

      .workflow-list {
        flex: 1;
        overflow-y: auto;
        height: calc(100% - 60px);

        .list {
          .item {
            padding: 10px 15px;
            cursor: pointer;
            border-radius: 6px;
            margin-bottom: 2px;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
            transition: background-color 0.2s ease;
            display: flex;
            flex-direction: column;
            gap: 4px;
            position: relative;

            &:hover:not(.disabled) {
              background-color: var(--cs-hover-bg-color);
            }

            &.active {
              background-color: var(--cs-active-bg-color);
              color: var(--el-color-primary);
            }

            &.disabled {
              cursor: not-allowed;
              opacity: 0.6;
            }

            .workflow-title {
              font-weight: 500;
              overflow: hidden;
              text-overflow: ellipsis;
            }

            .workflow-status {
              display: flex;
              align-items: center;
              gap: 6px;
              font-size: var(--cs-font-size-xs);
              color: var(--cs-text-color-secondary);

              .status-indicator {
                width: 6px;
                height: 6px;
                border-radius: 50%;

                &.running {
                  background-color: var(--el-color-warning);
                  animation: pulse 1.5s ease-in-out infinite;
                }

                &.completed {
                  background-color: var(--el-color-success);
                }

                &.paused {
                  background-color: var(--el-color-info);
                }

                &.error {
                  background-color: var(--el-color-danger);
                }
              }
            }

            .icons {
              position: absolute;
              right: 10px;
              top: 50%;
              transform: translateY(-50%);
              display: flex;
              gap: 4px;
              opacity: 0;
              transition: opacity 0.2s ease;

              .icon {
                display: flex;
                align-items: center;
                justify-content: center;
                width: 24px;
                height: 24px;
                border-radius: var(--cs-border-radius-round);
                cursor: pointer;
                color: var(--cs-text-color-secondary);

                &:hover {
                  background-color: var(--cs-bg-color-light);
                  color: var(--cs-text-color-primary);
                }

                .cs {
                  font-size: var(--cs-font-size-sm);
                }
              }
            }

            &:hover .icons {
              opacity: 1;
            }
          }
        }
      }
    }

    .main-container {
      display: flex;
      flex-direction: column;
      flex: 1;
      overflow: hidden;
      height: 100%;

      .messages {
        flex: 1;
        overflow-y: auto;
        padding: 15px;
        scroll-behavior: smooth;

        .message {
          display: flex;
          margin-bottom: 20px;
          position: relative;

          .avatar {
            flex-shrink: 0;
            width: 32px;
            height: 32px;
            display: flex;
            align-items: center;
            justify-content: center;
            margin-right: 12px;
            margin-top: 2px;
            border-radius: 50%;
            background-color: var(--cs-bg-color);
            border: 1px solid var(--cs-border-color);

            .user-icon {
              color: var(--el-color-primary);
              font-size: 18px;
            }
          }

          .content-container {
            flex: 1;
            min-width: 0;
            max-width: 90%;
          }

          &.user {
            justify-content: flex-end;

            .avatar {
              display: none; // Removing avatar as requested for a cleaner look if desired, or keep only for user
            }

            .content {
              display: flex;
              flex-direction: row-reverse;
              align-items: flex-start;
              gap: 8px;

              &:hover {
                .msg-ops {
                  opacity: 1;
                }
              }

              .simple-text {
                background-color: var(--el-color-primary-light-9);
                color: var(--cs-text-color-primary);
                padding: 10px 16px;
                border-radius: 18px 2px 18px 18px;
                max-width: 100%;
                border: 1px solid var(--el-color-primary-light-7);
                margin: 0;
                font-family: inherit;
                line-height: 1.6;
                white-space: pre-wrap;
              }

              .msg-ops {
                opacity: 0;
                transition: opacity 0.2s ease;
                display: flex;
                align-items: center;
                margin-top: 8px;

                .op-icon {
                  display: flex;
                  align-items: center;
                  justify-content: center;
                  width: 24px;
                  height: 24px;
                  border-radius: 50%;
                  cursor: pointer;
                  color: var(--cs-text-color-secondary);

                  &:hover {
                    color: var(--el-color-danger);
                  }
                }
              }
            }
          }

          &.assistant,
          &.tool {
            position: relative;

            &:hover {
              .msg-ops.floating {
                opacity: 1;
              }
            }

            .ai-content {
              background-color: transparent;
              padding: 0;
              font-size: var(--cs-font-size-md);
              line-height: 2;

              // CLI Style Tool Calls
              .cli-tool-calls-container {
                margin-bottom: 8px;
              }

              .cli-tool-call {
                font-family: var(--cs-font-family-mono, monospace);
                font-size: 13px;
                line-height: 1.5;
                margin-bottom: 8px;
                display: block; // Force block container

                &.expandable {
                  cursor: pointer;
                }

                .tool-line {
                  display: flex;
                  flex-direction: row; // Explicit horizontal
                  align-items: center;
                  white-space: nowrap;
                  width: 100%;
                  gap: 8px;

                  &.title-wrap {
                    user-select: none;
                    margin-bottom: 2px;

                    .status-dot {
                      color: var(--el-color-success);
                      width: 16px;
                      display: flex;
                      align-items: center;
                      justify-content: center;
                      flex-shrink: 0;

                      &.error {
                        color: var(--el-color-danger);
                      }
                    }

                    .tool-title {
                      color: var(--cs-text-color-primary);
                      font-size: var(--cs-font-size);
                      font-weight: 600;
                      white-space: nowrap;
                      overflow: hidden;
                      text-overflow: ellipsis;
                      flex: 1; // Take remaining space
                    }
                  }

                  &.summary {
                    color: var(--cs-text-color-secondary);
                    padding-left: 4px;
                    align-items: flex-start; // Keep icon at top

                    .corner-icon {
                      font-size: 16px;
                      width: 16px;
                      display: inline-block;
                      text-align: center;
                      flex-shrink: 0;
                      margin-top: -2px;
                    }

                    .summary-text {
                      font-size: 12px;
                      white-space: nowrap;
                      overflow: hidden;
                      text-overflow: ellipsis;
                      flex: 0 1 auto; // Auto-shrink based on content
                      line-height: 1.4;
                      padding-top: var(--cs-space-xs);
                    }

                    .expand-hint {
                      font-size: 11px;
                      opacity: 0.4;
                      margin-left: 4px; // Closer to text
                      flex: 0 0 auto; // Keep fixed width
                      line-height: 1.4;
                      padding-top: var(--cs-space-xs); // Match summary-text padding
                    }
                  }
                }

                &.error {

                  .status-dot,
                  .summary-text {
                    color: var(--el-color-danger);
                  }
                }

                &.pending {

                  .status-dot,
                  .tool-title {
                    color: var(--cs-text-color-placeholder);
                  }
                }

                .tool-detail {
                  margin-top: 8px;
                  margin-left: 20px;
                  padding: 12px;
                  background-color: var(--cs-bg-color-light);
                  border-radius: var(--cs-border-radius-sm);
                  border-left: 2px solid var(--cs-border-color);
                  font-family: var(--cs-font-family-mono, monospace);

                  .raw-content {
                    margin: 0;
                    white-space: pre-wrap;
                    word-break: break-all;
                    font-size: 12px;
                    color: var(--cs-text-color-regular);
                    background: none;
                    border: none;
                    padding: 0;
                    max-height: 300px;
                    overflow: auto;
                  }
                }
              }

              // Thoughts
              .thought-content {
                margin-bottom: 12px;
                color: var(--cs-text-color-secondary);
                font-style: italic;
                font-size: 13px;
                line-height: 1.6;
                padding: 8px 12px;
                background-color: var(--cs-bg-color);
                border-radius: var(--cs-border-radius-sm);
                border-left: 3px solid var(--cs-border-color-light);
                white-space: pre-wrap;
              }

              .msg-ops-container {
                position: relative;
                height: 0;
                width: 100%;
              }

              .msg-ops.floating {
                position: absolute;
                right: 0;
                top: -20px;
                opacity: 0;
                transition: opacity 0.2s ease;
                display: flex;
                gap: 4px;
                z-index: 10;

                .op-icon {
                  background: var(--cs-bg-color);
                  border: 1px solid var(--cs-border-color);
                  border-radius: 50%;
                  width: 24px;
                  height: 24px;
                  display: flex;
                  align-items: center;
                  justify-content: center;
                  cursor: pointer;
                  color: var(--cs-text-color-secondary);

                  &:hover {
                    color: var(--el-color-danger);
                  }
                }
              }
            }
          }

          &.observe {
            opacity: 0.9;
            font-size: 0.95em;
          }
        }
      }

      .todo-list-wrapper {
        flex-shrink: 0;
        padding: 0 var(--cs-space) var(--cs-space-sm);
      }

      footer.input-container {
        flex-shrink: 0;
        background-color: transparent;
        padding: 0 var(--cs-space-sm) var(--cs-space-sm);
        height: unset;
        z-index: 1;
        position: relative;

        .slash-command-panel {
          position: absolute;
          bottom: calc(100% - 10px);
          left: var(--cs-space-sm);
          right: var(--cs-space-sm);
          background-color: var(--cs-bg-color);
          border: 1px solid var(--cs-border-color);
          border-radius: var(--cs-border-radius-lg);
          box-shadow: var(--el-box-shadow-light);
          max-height: 300px;
          overflow-y: auto;
          z-index: 100;
          padding: 4px;

          .command-item {
            padding: 8px 12px;
            cursor: pointer;
            border-radius: var(--cs-border-radius-sm);
            display: flex;
            flex-direction: column;
            gap: 2px;

            &:hover,
            &.active {
              background-color: var(--cs-hover-bg-color);
            }

            .command-name {
              font-weight: 600;
              font-size: 13px;
              color: var(--cs-color-primary);
            }

            .command-desc {
              font-size: 12px;
              color: var(--cs-text-color-secondary);
              white-space: nowrap;
              overflow: hidden;
              text-overflow: ellipsis;
            }
          }
        }

        .additional {
          display: flex;
          gap: 1px;
          margin-bottom: var(--cs-space-xs);

          .additional-item {
            display: flex;
            align-items: center;
            flex: 1;
            max-width: 50%;
            background-color: var(--cs-input-bg-color);
            border-radius: var(--cs-border-radius-xxl);
            padding: var(--cs-space-xs);
            box-sizing: border-box;

            .data {
              flex: 1;
              min-width: 0;

              .skill-item {
                padding: 0;
              }

              .message-text {
                padding-left: var(--cs-space);
                display: block;
                white-space: nowrap;
                overflow: hidden;
                text-overflow: ellipsis;
                color: var(--cs-text-color-secondary);
                font-size: var(--cs-font-size-sm);
                line-height: 1.5;
                position: relative;

                &:before {
                  position: absolute;
                  top: -3px;
                  left: 3px;
                }
              }
            }

            .close-btn {
              display: flex;
              align-items: center;
              justify-content: center;
              width: 24px;
              height: 24px;
              margin-left: var(--cs-space-xs);
              flex-shrink: 0;
              cursor: pointer;
              border-radius: var(--cs-border-radius-round);
              color: var(--cs-text-color-secondary);

              &:hover {
                background-color: var(--cs-bg-color-light);
              }
            }
          }
        }

        .input {
          display: flex;
          flex-direction: column;
          background-color: var(--cs-input-bg-color);
          border-radius: var(--cs-border-radius-lg);
          padding: var(--cs-space-sm) var(--cs-space) var(--cs-space-xs);

          .icons {
            display: flex;
            align-items: center;
            justify-content: center;
            padding: var(--cs-space-xs);
            cursor: pointer;
            gap: var(--cs-space-xs);

            .cs {
              font-size: var(--cs-font-size-xl) !important;
              color: var(--cs-text-color-secondary);

              &.small {
                font-size: var(--cs-font-size-md) !important;
              }

              &.cs-send:not(.disabled) {
                color: var(--cs-color-primary);
              }
            }

            label {
              font-size: var(--cs-font-size-sm);
              display: flex;
              align-items: center;
              justify-content: center;
              cursor: pointer;
              color: var(--cs-text-color-secondary);
              background-color: var(--cs-bg-color);
              border-radius: var(--cs-border-radius-lg);
              padding: var(--cs-space-xs) var(--cs-space-sm);
              border: 1px solid var(--cs-bg-color);

              &:not(.disabled):not(.default):hover,
              &.active {
                color: var(--cs-color-primary);

                .cs {
                  color: var(--cs-color-primary);
                }
              }

              &.active {
                border: 1px solid var(--cs-color-primary);
              }
            }
          }

          .el-textarea {
            flex-grow: 1;

            .el-textarea__inner {
              border: none;
              box-shadow: none;
              background: var(--cs-input-bg-color) !important;
              resize: none !important;
              color: var(--cs-text-color-primary);
              padding-left: var(--cs-space-xxs);
              padding-right: var(--cs-space-xxs);
            }
          }

          .input-footer {
            display: flex;
            flex-direction: row;
            align-items: center;
            justify-content: space-between;

            .footer-left {
              display: flex;
              flex-direction: row;
              justify-content: flex-start;
              align-items: center;

              .agent-selector-wrap {
                color: var(--cs-color-primary);
                background: var(--cs-bg-color);
                border: 1px solid var(--cs-color-primary);
                border-radius: var(--cs-border-radius-lg);
                padding: var(--cs-space-xs) var(--cs-space-sm);
                font-size: var(--cs-font-size-md);

                &.disabled {
                  border-color: var(--cs-border-color);
                  background: none;
                }
              }

              .allowed-paths-wrap {
                margin-left: 8px;

                .paths-summary {
                  display: flex;
                  align-items: center;
                  gap: 4px;
                  font-size: 12px;
                  color: var(--cs-text-color-secondary);
                  background-color: var(--cs-input-bg-color);
                  border: 1px solid var(--cs-border-color);
                  border-radius: var(--cs-border-radius-lg);
                  padding: 4px 8px;
                  cursor: pointer;
                  transition: all 0.2s ease;
                  min-width: 80px; // Ensure visibility

                  &.empty {
                    border-style: dashed;
                    opacity: 0.8;
                  }

                  &:hover {
                    border-color: var(--cs-color-primary);
                    color: var(--cs-color-primary);
                  }

                  .path-text {
                    max-width: 100px;
                    overflow: hidden;
                    text-overflow: ellipsis;
                    white-space: nowrap;
                  }

                  .path-count {
                    font-size: 10px;
                    background-color: var(--cs-color-primary-light-8);
                    color: var(--cs-color-primary);
                    padding: 0 4px;
                    border-radius: 4px;
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}

.paths-popover {
  .paths-detail {
    .paths-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 12px;
      font-weight: 600;
      font-size: 13px;
      color: var(--cs-text-color-primary);
    }

    .paths-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
      max-height: 200px;
      overflow-y: auto;

      .path-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        background-color: var(--cs-bg-color-light);
        padding: 4px 8px;
        border-radius: 4px;
        font-size: 12px;

        .path-name {
          flex: 1;
          overflow: hidden;
          text-overflow: ellipsis;
          white-space: nowrap;
          color: var(--cs-text-color-regular);
        }

        .path-ops {
          color: var(--cs-text-color-secondary);
          cursor: pointer;

          &:hover {
            color: var(--el-color-danger);
          }
        }
      }

      .empty-paths {
        font-size: 12px;
        color: var(--cs-text-color-placeholder);
        font-style: italic;
        text-align: center;
        padding: 12px 0;
      }
    }
  }
}

.pin-btn {
  border-radius: var(--cs-border-radius-xs);
  color: var(--cs-text-color-secondary);

  &:hover .cs {
    color: var(--cs-color-primary) !important;
  }

  .cs {
    font-size: var(--cs-font-size-md) !important;
    transform: rotate(45deg);
    transition: all 0.3s ease-in-out;
  }

  &.active {
    .cs {
      color: var(--cs-color-primary);
      transform: rotate(0deg);
    }
  }
}

@keyframes pulse {
  0% {
    opacity: 1;
  }

  50% {
    opacity: 0.5;
  }

  100% {
    opacity: 1;
  }
}

.rotating {
  animation: cs-rotate 2s linear infinite;
  display: inline-block;
  margin-right: 4px;
}

@keyframes cs-rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
</style>
