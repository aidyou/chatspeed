<template>
  <Teleport to="body">
    <!-- Large panel -->
    <div
      v-if="isVisible && hasData"
      ref="panelRef"
      class="status-panel"
      :class="{ collapsed: isCollapsed, dragging: isDragging }"
      :style="panelStyle">
      <!-- Drag handle/header -->
      <div class="panel-header upperLayer" @mousedown="startDrag" @touchstart="startDrag">
        <div class="header-left">
          <cs name="list" size="14px" class="drag-icon" />
          <span v-if="!isCollapsed" class="header-title">{{
            t('workflow.statusPanel.title')
          }}</span>
        </div>
        <div class="header-actions">
          <span v-if="isCollapsed" class="collapsed-progress">{{ progressPercent }}%</span>
          <span v-if="!isCollapsed && todoList.length > 0" class="task-count">
            {{ completedCount }}/{{ todoList.length }}
          </span>
          <div class="action-btn" @click.stop="toggleCollapse">
            <cs :name="isCollapsed ? 'fullscreen-off' : 'minimize'" size="14px" />
          </div>
          <div class="action-btn close-btn" @click.stop="hidePanel">
            <cs name="close" size="12px" />
          </div>
        </div>
      </div>

      <!-- Expanded content -->
      <div v-if="!isCollapsed" class="panel-body">
        <div class="panel-tabs">
          <button
            class="tab-btn"
            :class="{ active: activeTab === 'main' }"
            @click="activeTab = 'main'">
            {{ t('workflow.statusPanel.mainAgent') || 'Main' }}
          </button>
          <button
            class="tab-btn"
            :class="{ active: activeTab === 'sub' }"
            @click="activeTab = 'sub'">
            {{ t('workflow.statusPanel.subAgents') || 'Sub-agents' }}
            <span v-if="childAgentSummaries.length > 0" class="tab-badge">{{
              childAgentSummaries.length
            }}</span>
          </button>
        </div>

        <template v-if="activeTab === 'main'">
          <!-- Context Usage section -->
          <div class="section progress-section">
            <div class="section-header">
              <cs name="skill-piechart" size="14px" />
              <span>{{ t('workflow.statusPanel.contextUsage') || 'Context Usage' }}</span>
            </div>
            <div class="progress-bar-container">
              <div class="progress-bar">
                <div
                  class="progress-fill context-progress"
                  :style="{ width: `${contextUsagePercent}%` }"
                  :class="contextUsageStatusClass" />
              </div>
              <span class="progress-text">{{ contextUsagePercent }}%</span>
            </div>
            <div class="usage-details" v-if="totalTokens > 0">
              {{ formatNumber(totalTokens) }} / {{ formatNumber(maxContexts) }} tokens
            </div>
          </div>

          <!-- Progress section -->
          <div v-if="todoList.length > 0" class="section progress-section">
            <div class="section-header">
              <cs name="skill-terminal" size="14px" />
              <span>{{ t('workflow.statusPanel.progress') }}</span>
            </div>
            <div class="progress-bar-container">
              <div class="progress-bar">
                <div
                  class="progress-fill task-progress"
                  :style="{ width: `${progressPercent}%` }"
                  :class="progressStatusClass" />
              </div>
              <span class="progress-text">{{ progressPercent }}%</span>
            </div>
          </div>

          <!-- Todo section -->
          <div v-if="todoList.length > 0" class="section">
            <div class="section-header">
              <cs name="list" size="14px" />
              <span>{{ t('workflow.statusPanel.todos') }}</span>
            </div>
            <ul class="todo-list">
              <li
                v-for="item in displayedTodoList"
                :key="item.id"
                :class="['todo-item', item.status]">
                <cs
                  :name="getStatusIcon(item.status)"
                  :class="{ 'cs-spin': item.status === 'in_progress' }"
                  size="14px"
                  class="todo-icon" />
                <span class="todo-text" :title="item.subject || item.title">
                  {{ item.subject || item.title }}
                </span>
              </li>
            </ul>
            <div
              v-if="todoList.length > 10"
              class="more-indicator clickable"
              @click="isTodoExpanded = !isTodoExpanded">
              {{
                isTodoExpanded
                  ? t('common.collapse')
                  : `+${todoList.length - 10} ${t('common.more')}`
              }}
            </div>
          </div>

          <!-- Recent operations section -->
          <div v-if="recentOperations.length > 0" class="section">
            <div class="section-header">
              <cs name="tool" size="14px" />
              <span>{{ t('workflow.statusPanel.recentOps') }}</span>
              <span class="section-meta"
                >{{ t('workflow.statusPanel.totalCalls') || 'Total' }}: {{ totalToolCalls }}</span
              >
            </div>
            <ul class="operations-list">
              <li
                v-for="(op, index) in recentOperations"
                :key="index"
                :class="['op-item', op.status, op.toolType]">
                <div class="op-main">
                  <cs :name="op.icon" size="14px" class="op-icon" />
                  <span class="op-name" :title="op.fullText">{{ op.name }}</span>
                </div>
                <cs
                  v-if="op.status === 'running'"
                  name="loading"
                  size="12px"
                  class="op-status cs-spin" />
                <cs
                  v-else-if="op.status === 'success'"
                  name="check"
                  size="12px"
                  class="op-status success" />
                <cs
                  v-else-if="op.status === 'error'"
                  name="error"
                  size="12px"
                  class="op-status error" />
              </li>
            </ul>
          </div>
        </template>

        <!-- Sub agents tab -->
        <div v-if="activeTab === 'sub' && childAgentSummaries.length > 0" class="section">
          <div class="section-header">
            <cs name="agent" size="14px" />
            <span>{{ t('workflow.statusPanel.childAgents') || 'Child Agents' }}</span>
            <span class="section-meta">{{
              childAgentTotalCount > childAgentSummaries.length
                ? `${childAgentSummaries.length}/${childAgentTotalCount}`
                : childAgentSummaries.length
            }}</span>
          </div>
          <ul class="child-agent-list">
            <li
              v-for="child in childAgentSummaries"
              :key="child.id"
              class="child-agent-item clickable"
              :class="child.status"
              @click="jumpToChildMessage(child)">
              <div class="child-main">
                <div class="child-header">
                  <span class="child-agent" :title="child.agentName">{{ child.agentName }}</span>
                  <span class="child-status-pill" :class="child.status">
                    <cs
                      v-if="child.status === 'running'"
                      name="loading"
                      size="10px"
                      class="cs-spin child-status-pill-icon" />
                    {{ getChildStatusLabel(child.status) }}
                  </span>
                </div>
                <span class="child-task" :title="child.task">{{ child.task }}</span>
                <div class="child-metrics">
                <span class="child-metric-label">{{
                    translateOrFallback('workflow.statusPanel.latestDynamic', 'Latest')
                  }}</span>
                  <span class="child-summary" :title="child.summary">{{ child.summary }}</span>
                </div>
                <div class="child-metrics child-stats">
                  <span class="child-tools" :title="`${child.toolCalls} tool calls`"
                    >Tools {{ child.toolCalls }}</span
                  >
                  <span v-if="child.contextPercent !== null" class="child-context"
                    >Ctx {{ child.contextPercent }}%</span
                  >
                </div>
              </div>
            </li>
          </ul>
        </div>

        <div v-if="activeTab === 'sub' && childAgentSummaries.length === 0" class="empty-state">
          <cs name="agent" size="28px" />
          <span>{{ t('workflow.statusPanel.noSubAgents') || 'No sub-agents yet' }}</span>
        </div>

        <!-- Empty state -->
        <div
          v-if="activeTab === 'main' && todoList.length === 0 && recentOperations.length === 0"
          class="empty-state">
          <cs name="file" size="32px" />
          <span>{{ t('workflow.statusPanel.empty') }}</span>
        </div>
      </div>
    </div>

    <!-- Trigger button (small circle) -->
    <div
      v-else-if="hasData"
      ref="triggerRef"
      class="status-panel-trigger"
      :style="triggerStyle"
      @click="onTriggerClick">
      <div
        class="trigger-drag-area"
        @mousedown.stop.prevent="startTriggerDrag"
        @touchstart.stop.prevent="startTriggerDrag"></div>
      <cs name="list" size="18px" />
      <span v-if="progressPercent > 0" class="trigger-badge">{{ progressPercent }}%</span>
    </div>
  </Teleport>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { resolveWorkflowToolIcon } from '@/composables/workflow/toolIcons'
import { invokeWrapper } from '@/libs/tauri'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()

// Panel state
const isVisible = ref(true)
const isCollapsed = ref(false)
const isDragging = ref(false)
const isTodoExpanded = ref(false)
const activeTab = ref('main')

// Position: use left/top for unified storage
const position = ref({ x: 0, y: 0 })
const isPositioned = ref(false)

// Edge distance (for smart positioning during resize)
const edgeDistance = ref({ right: 20, bottom: 220 })

// Drag offset
const dragOffset = ref({ x: 0, y: 0 })

// Drag flag (to distinguish between click and drag)
const hasDragged = ref(false)

// DOM references
const panelRef = ref(null)
const triggerRef = ref(null)

// Get data from store
const todoList = computed(() => workflowStore.todoList)
const displayedTodoList = computed(() => {
  if (isTodoExpanded.value) return todoList.value
  return todoList.value.slice(0, 10)
})
const messages = computed(() => workflowStore.messages)
const isRunning = computed(() => workflowStore.isRunning)
const toolLedger = computed(() => workflowStore.toolList || [])

// Panel dimensions
const PANEL_WIDTH = 280
const PANEL_HEIGHT = 200
const COLLAPSED_WIDTH = 140
const COLLAPSED_HEIGHT = 40
const TRIGGER_SIZE = 44
const CHILD_AGENT_LIMIT = 5
const childSnapshotProgress = ref(new Map())
const MIN_TOP_GAP = 5
const DRAG_START_THRESHOLD = 4

const getSafeTopInset = () => {
  if (typeof window === 'undefined') return 37
  const rootStyle = getComputedStyle(document.documentElement)
  const raw = rootStyle.getPropertyValue('--cs-titlebar-height').trim()
  const titlebarHeight = Number.parseFloat(raw || '32')
  return (Number.isFinite(titlebarHeight) ? titlebarHeight : 32) + MIN_TOP_GAP
}

// Calculate progress percentage
const progressPercent = computed(() => {
  if (todoList.value.length === 0) return 0
  const completed = todoList.value.filter(
    item =>
      item.status === 'completed' || item.status === 'failed' || item.status === 'data_missing'
  ).length
  return Math.round((completed / todoList.value.length) * 100)
})

const completedCount = computed(() => {
  return todoList.value.filter(
    item =>
      item.status === 'completed' || item.status === 'failed' || item.status === 'data_missing'
  ).length
})

const progressStatusClass = computed(() => {
  if (progressPercent.value === 100) return 'complete'
  if (progressPercent.value >= 60) return 'good'
  if (progressPercent.value >= 30) return 'medium'
  return 'start'
})

// Calculate Context Usage
const getModelContextSize = modelConfig => {
  if (!modelConfig || typeof modelConfig !== 'object') return null
  const rawValue = modelConfig.contextSize ?? modelConfig.context_size
  return typeof rawValue === 'number' && rawValue > 0 ? rawValue : null
}

const maxContexts = computed(() => {
  const runtimeMax = workflowStore.currentWorkflow?.executionContext?.maxContextTokens
  if (typeof runtimeMax === 'number' && runtimeMax > 0) {
    return runtimeMax
  }

  const workflowConfig = workflowStore.currentWorkflow?.agentConfig || {}
  const workflowModels = workflowConfig.models || {}
  const phase = String(workflowConfig.phase || '').toLowerCase()
  const phaseModel = phase === 'planning' ? workflowModels.plan : workflowModels.act
  const phaseLimit = getModelContextSize(phaseModel)
  if (phaseLimit) return phaseLimit

  const fallbackLimit =
    getModelContextSize(workflowModels.act) || getModelContextSize(workflowModels.plan)
  if (fallbackLimit) return fallbackLimit

  const agentId = workflowStore.currentWorkflow?.agentId
  if (!agentId) return 128000
  const agent = agentStore.agents.find(a => a.id === agentId)
  return agent?.maxContexts || 128000
})

const totalTokens = computed(() => {
  const currentContextTokens = workflowStore.currentWorkflow?.executionContext?.currentContextTokens
  if (typeof currentContextTokens === 'number' && currentContextTokens >= 0) {
    return currentContextTokens
  }

  // Find the most recent message with usage information
  const lastAssistantMsg = [...messages.value]
    .reverse()
    .find(
      m =>
        m.role === 'assistant' &&
        (m.metadata?.usage ||
          m.metadata?.tokens ||
          m.metadata?.input_tokens ||
          m.metadata?.prompt_tokens)
    )

  if (!lastAssistantMsg) return 0

  const meta = lastAssistantMsg.metadata
  // 1. Try ChatMetadata style (nested tokens object)
  if (meta.tokens) {
    return meta.tokens.total || meta.tokens.prompt + meta.tokens.completion || 0
  }

  // 2. Try usage object style
  if (meta.usage) {
    const u = meta.usage
    return (
      u.total_tokens ||
      (u.input_tokens || u.prompt_tokens || 0) + (u.output_tokens || u.completion_tokens || 0) ||
      0
    )
  }

  // 3. Fallback to flattened style
  const input = meta.input_tokens || meta.prompt_tokens || 0
  const output = meta.output_tokens || meta.completion_tokens || 0
  const total = meta.total_tokens || input + output
  return total || 0
})

const contextUsagePercent = computed(() => {
  if (maxContexts.value <= 0) return 0
  const percent = Math.round((totalTokens.value / maxContexts.value) * 100)
  return Math.min(percent, 100)
})

const contextUsageStatusClass = computed(() => {
  if (contextUsagePercent.value >= 90) return 'complete'
  if (contextUsagePercent.value >= 70) return 'good'
  if (contextUsagePercent.value >= 40) return 'medium'
  return 'start'
})

const formatNumber = num => {
  if (!num) return '0'
  return new Intl.NumberFormat().format(num)
}

const translateOrFallback = (key, fallback) => {
  const translated = t(key)
  return !translated || translated === key ? fallback : translated
}

// Helper to remove <SYSTEM_REMINDER>...</SYSTEM_REMINDER> tags
const removeSystemReminder = content => {
  if (!content) return ''
  return content.replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '').trim()
}

const getToolInfo = (name, metadata = {}) => {
  const iconMap = {
    read_file: { icon: resolveWorkflowToolIcon('read_file', 'file'), toolType: 'tool-file' },
    write_file: { icon: resolveWorkflowToolIcon('write_file', 'file'), toolType: 'tool-file' },
    edit_file: { icon: resolveWorkflowToolIcon('edit_file', 'edit'), toolType: 'tool-file' },
    list_dir: { icon: resolveWorkflowToolIcon('list_dir', 'folder'), toolType: 'tool-file' },
    glob: { icon: resolveWorkflowToolIcon('glob', 'search'), toolType: 'tool-file' },
    grep: { icon: resolveWorkflowToolIcon('grep', 'search'), toolType: 'tool-file' },
    web_fetch: { icon: resolveWorkflowToolIcon('web_fetch', 'link'), toolType: 'tool-network' },
    web_search: { icon: resolveWorkflowToolIcon('web_search', 'search'), toolType: 'tool-network' },
    bash: { icon: resolveWorkflowToolIcon('bash', 'terminal'), toolType: 'tool-system' },
    todo_create: { icon: resolveWorkflowToolIcon('todo_create', 'add'), toolType: 'tool-todo' },
    todo_update: { icon: resolveWorkflowToolIcon('todo_update', 'check'), toolType: 'tool-todo' },
    todo_list: { icon: resolveWorkflowToolIcon('todo_list', 'list'), toolType: 'tool-todo' },
    todo_get: { icon: resolveWorkflowToolIcon('todo_get', 'list'), toolType: 'tool-todo' },
    submit_plan: {
      icon: resolveWorkflowToolIcon('submit_plan', 'skill-plan'),
      toolType: 'tool-todo'
    },
    complete_workflow_with_summary: { icon: 'check-circle', toolType: 'tool-todo' }
  }

  const info = iconMap[name] || {
    icon: resolveWorkflowToolIcon(name, 'tool'),
    toolType: 'tool-system'
  }

  return {
    ...info,
    shortName: metadata.title || name.replace(/_/g, ' ')
  }
}

const toolMessagesAll = computed(() => {
  return messages.value.filter(m => {
    if (m.role !== 'tool') return false
    const name = (m.metadata?.tool_name || m.metadata?.tool_call?.name || '').toLowerCase()
    if (name === 'complete_workflow_with_summary' || name === 'answer_user') return false
    return true
  })
})

const ledgerStatusToPanelStatus = status => {
  if (status === 'final_error' || status === 'rejected') return 'error'
  if (status === 'final_success') return 'success'
  if (status === 'approved_running' || status === 'pending') return 'running'
  return 'success'
}

// Calculate recent operations from the unified task ledger when available.
const recentOperations = computed(() => {
  if (toolLedger.value.length > 0) {
    return toolLedger.value
      .slice(-3)
      .reverse()
      .map(tool => {
        const meta = {
          title: tool.title,
          summary: tool.summary
        }
        const { icon, toolType, shortName } = getToolInfo(tool.toolName || 'Tool', meta)
        return {
          name: shortName,
          fullText: removeSystemReminder(tool.summary || tool.toolName || ''),
          icon,
          toolType,
          status: ledgerStatusToPanelStatus(tool.status),
          raw: tool
        }
      })
  }

  return toolMessagesAll.value
    .slice(-3)
    .reverse()
    .map(m => {
      const meta = m.metadata || {}
      const toolCall = meta.tool_call || {}
      const func = toolCall.function || toolCall
      const name = func.name || toolCall.name || meta.tool_name || 'Tool'
      const executionStatus = meta.execution_status || ''

      let status = 'success'
      if (
        m.isError ||
        meta.is_error ||
        executionStatus === 'failed' ||
        executionStatus === 'rejected'
      ) {
        status = 'error'
      } else if (executionStatus === 'running' || executionStatus === 'pending_approval') {
        status = 'running'
      }

      const { icon, toolType, shortName } = getToolInfo(name, meta)

      return {
        name: shortName,
        fullText: removeSystemReminder(meta.summary || name),
        icon,
        toolType,
        status,
        raw: m
      }
    })
})

const totalToolCalls = computed(() => {
  return toolLedger.value.length > 0 ? toolLedger.value.length : toolMessagesAll.value.length
})

const extractSubAgentIdFromMessage = message => {
  const meta = message?.metadata || {}
  if (meta.sub_agent_id || meta.subAgentId) return meta.sub_agent_id || meta.subAgentId
  if (meta.data?.sub_agent_id || meta.data?.subAgentId) return meta.data.sub_agent_id || meta.data.subAgentId
  if ((meta.tool_name || '').toLowerCase() !== 'sub_agent_run') return null

  try {
    const parsed = JSON.parse(message.message || '{}')
    return parsed.task_id || parsed.taskId || null
  } catch {
    return null
  }
}

const truncateSummary = (value, limit = 60) => {
  const text = removeSystemReminder(String(value || '')).trim()
  if (!text) return ''
  return text.length > limit ? `${text.slice(0, limit)}...` : text
}

const normalizeChildPanelStatus = (status, isError = false) => {
  const normalized = String(status || '').toLowerCase()
  if (isError || ['failed', 'error', 'cancelled', 'interrupted'].includes(normalized))
    return 'failed'
  if (['completed', 'success'].includes(normalized)) return 'success'
  if (['running', 'thinking', 'executing', 'waiting', 'pending'].includes(normalized))
    return 'running'
  return 'pending'
}

const contextPercentFromProgress = progress => {
  const current = progress?.currentContextTokens ?? progress?.current_context_tokens
  const max = progress?.maxContextTokens ?? progress?.max_context_tokens
  if (typeof current !== 'number' || typeof max !== 'number' || max <= 0) return null
  return Math.min(100, Math.round((current / max) * 100))
}

const getChildStatusLabel = status => {
  const normalized = String(status || '').toLowerCase()
  if (normalized === 'success') return 'Done'
  if (normalized === 'failed') return 'Failed'
  if (normalized === 'running') return 'Running'
  return 'Pending'
}

const buildSubAgentProgressFromSnapshot = (id, snapshot) => {
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

const childSessionIdsFromSource = computed(() => {
  const ctx = workflowStore.currentWorkflow?.executionContext || {}
  const sessionsFromContext = ctx.subAgentSessions || ctx.sub_agent_sessions || []
  const waitingTaskId = ctx.waitingOnSubAgentId || ctx.waiting_on_sub_agent_id || null
  const sessionsFromMessages = messages.value
    .map(m => extractSubAgentIdFromMessage(m))
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

const childSessionIds = computed(() => {
  return Array.from(
    new Set([...childSessionIdsFromSource.value, ...Array.from(childSnapshotProgress.value.keys())])
  )
})

const childAgentTotalCount = computed(() => childSessionIds.value.length)

const childAgentSummariesAll = computed(() => {
  const ids = childSessionIds.value
  if (!ids.length) return []

  return ids.map(id => {
    const ctx = workflowStore.currentWorkflow?.executionContext || {}
    const childWorkflow = workflowStore.workflows.find(w => w.id === id)
    const related = messages.value.filter(m => {
      return extractSubAgentIdFromMessage(m) === id
    })
    const last = related[related.length - 1]
    const lastIndex = last ? messages.value.lastIndexOf(last) : -1
    const eventProgress = workflowStore.subAgentProgress?.get?.(id)
    const snapshotProgress = childSnapshotProgress.value.get(id)
    const progress = {
      ...(snapshotProgress || {}),
      ...(eventProgress || {})
    }
    let status = (ctx.waitingOnSubAgentId || ctx.waiting_on_sub_agent_id) === id ? 'running' : 'pending'
    let summary = t('workflow.statusPanel.childRunning') || 'Running'
    let toolCalls = 0
    const workflowAgentName = childWorkflow?.agentName
      || childWorkflow?.agent_name
      || agentStore.agents.find(agent => agent.id === (childWorkflow?.agentId || childWorkflow?.agent_id))?.name
      || null
    let agentName = progress.agentName || progress.agent_name || workflowAgentName || 'Sub-agent'
    let task = progress.task || childWorkflow?.userQuery || childWorkflow?.user_query || childWorkflow?.title || id

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
        meta.execution_status ||
        meta.sub_agent_status ||
        observationData.execution_status ||
        ''
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
      } else if (executionStatus === 'waiting' || executionStatus === 'running') {
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
      const s = String(childWorkflow.status).toLowerCase()
      if (s === 'completed') status = 'success'
      if (s === 'error' || s === 'failed' || s === 'cancelled') status = 'failed'
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
      lastSeen: Math.max(lastIndex, progress.updatedAtMs || progress.updated_at_ms || 0)
    }
  })
})

const childAgentSummaries = computed(() => {
  return [...childAgentSummariesAll.value]
    .sort((a, b) => b.lastSeen - a.lastSeen)
    .slice(0, CHILD_AGENT_LIMIT)
})

const refreshChildSnapshots = async () => {
  const ids = childSessionIdsFromSource.value.slice(-CHILD_AGENT_LIMIT)
  if (!ids.length) {
    childSnapshotProgress.value = new Map()
    return
  }

  const next = new Map()
  await Promise.all(
    ids.map(async id => {
      try {
        const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: id })
        next.set(id, buildSubAgentProgressFromSnapshot(id, snapshot))
      } catch (error) {
        console.warn(`[Workflow] Failed to load child task snapshot ${id}:`, error)
      }
    })
  )
  childSnapshotProgress.value = next
}

const escapeSelectorValue = value => {
  if (window.CSS?.escape) return window.CSS.escape(value)
  return String(value).replace(/["\\]/g, '\\$&')
}

const jumpToChildMessage = child => {
  if (!child?.id) return
  const selector = `[data-child-task-id="${escapeSelectorValue(child.id)}"]`
  const matches = Array.from(document.querySelectorAll(selector))
  const target = matches[matches.length - 1]
  if (!target) return
  target.scrollIntoView({ behavior: 'smooth', block: 'center' })
  if (typeof target.animate === 'function') {
    target.animate(
      [{ backgroundColor: 'rgba(64, 158, 255, 0.18)' }, { backgroundColor: 'transparent' }],
      { duration: 1200, easing: 'ease-out' }
    )
  }
}

// Hide panel when there's no data to show
const hasData = computed(() => {
  return (
    todoList.value.length > 0 ||
    recentOperations.value.length > 0 ||
    childAgentSummaries.value.length > 0
  )
})

const getStatusIcon = status => {
  switch (status) {
    case 'completed':
      return 'check'
    case 'in_progress':
      return 'loading'
    case 'failed':
      return 'error'
    case 'data_missing':
      return 'error'
    default:
      return 'uncheck'
  }
}

// Panel style - use right/bottom positioning by default
const panelStyle = computed(() => {
  if (!isPositioned.value) {
    return {
      right: '20px',
      bottom: '220px',
      left: 'auto',
      top: 'auto'
    }
  }

  // Use left/top
  return {
    left: `${position.value.x}px`,
    top: `${position.value.y}px`,
    right: 'auto',
    bottom: 'auto'
  }
})

// Trigger button style
const triggerStyle = computed(() => {
  if (!isPositioned.value) {
    return {
      right: '20px',
      bottom: '220px',
      left: 'auto',
      top: 'auto'
    }
  }

  return {
    left: `${position.value.x}px`,
    top: `${position.value.y}px`,
    right: 'auto',
    bottom: 'auto'
  }
})

// Large panel drag
const startDrag = e => {
  if (e.target.closest('.action-btn')) return
  e.preventDefault()

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const rect = panelRef.value?.getBoundingClientRect()
  if (rect) {
    dragOffset.value = {
      x: clientX - rect.left,
      y: clientY - rect.top
    }
  }

  hasDragged.value = false
  isDragging.value = true

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
  document.addEventListener('touchmove', onDrag)
  document.addEventListener('touchend', stopDrag)
}

const onDrag = e => {
  if (!isDragging.value) return
  e.preventDefault()

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const deltaX = clientX - (position.value.x + dragOffset.value.x)
  const deltaY = clientY - (position.value.y + dragOffset.value.y)
  if (!hasDragged.value && Math.hypot(deltaX, deltaY) < DRAG_START_THRESHOLD) {
    return
  }
  hasDragged.value = true

  const newX = clientX - dragOffset.value.x
  const newY = clientY - dragOffset.value.y

  // Use different dimensions based on current state
  const width = isCollapsed.value ? COLLAPSED_WIDTH : PANEL_WIDTH
  const height = isCollapsed.value ? COLLAPSED_HEIGHT : panelRef.value?.offsetHeight || 250
  const bottomReserved = isCollapsed.value ? 0 : 150
  const topInset = getSafeTopInset()

  position.value = {
    x: Math.max(0, Math.min(newX, window.innerWidth - width)),
    y: Math.max(topInset, Math.min(newY, window.innerHeight - height - bottomReserved))
  }
}

const stopDrag = () => {
  isDragging.value = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.removeEventListener('touchmove', onDrag)
  document.removeEventListener('touchend', stopDrag)

  if (!hasDragged.value) {
    return
  }

  isPositioned.value = true

  // Calculate and save edge distance
  const width = isCollapsed.value ? COLLAPSED_WIDTH : PANEL_WIDTH
  const height = isCollapsed.value ? COLLAPSED_HEIGHT : panelRef.value?.offsetHeight || 250
  edgeDistance.value = {
    right: window.innerWidth - position.value.x - width,
    bottom: window.innerHeight - position.value.y - height
  }

  savePosition()
}

// Trigger button drag
const startTriggerDrag = e => {
  hasDragged.value = false
  e.preventDefault()

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  // Get current position
  const rect = triggerRef.value?.getBoundingClientRect()
  if (rect) {
    // If position hasn't been set, calculate current position and set it
    if (!isPositioned.value) {
      position.value = {
        x: rect.left,
        y: rect.top
      }
      isPositioned.value = true
    }

    dragOffset.value = {
      x: clientX - rect.left,
      y: clientY - rect.top
    }
  }

  isDragging.value = true

  document.addEventListener('mousemove', onTriggerDrag)
  document.addEventListener('mouseup', stopTriggerDrag)
  document.addEventListener('touchmove', onTriggerDrag)
  document.addEventListener('touchend', stopTriggerDrag)
}

const onTriggerDrag = e => {
  if (!isDragging.value) return
  e.preventDefault()

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY
  const deltaX = clientX - (position.value.x + dragOffset.value.x)
  const deltaY = clientY - (position.value.y + dragOffset.value.y)
  if (!hasDragged.value && Math.hypot(deltaX, deltaY) < DRAG_START_THRESHOLD) {
    return
  }
  hasDragged.value = true

  const newX = clientX - dragOffset.value.x
  const newY = clientY - dragOffset.value.y
  const topInset = getSafeTopInset()

  // Boundary limit - small circle 44x44
  position.value = {
    x: Math.max(0, Math.min(newX, window.innerWidth - TRIGGER_SIZE)),
    y: Math.max(topInset, Math.min(newY, window.innerHeight - TRIGGER_SIZE))
  }
}

const stopTriggerDrag = () => {
  document.removeEventListener('mousemove', onTriggerDrag)
  document.removeEventListener('mouseup', stopTriggerDrag)
  document.removeEventListener('touchmove', onTriggerDrag)
  document.removeEventListener('touchend', stopTriggerDrag)

  // Save position and edge distance
  if (hasDragged.value) {
    edgeDistance.value = {
      right: window.innerWidth - position.value.x - TRIGGER_SIZE,
      bottom: window.innerHeight - position.value.y - TRIGGER_SIZE
    }
    savePosition()
  }

  isDragging.value = false
}

// Trigger button click
const onTriggerClick = () => {
  // Only restore panel if not dragged
  if (!hasDragged.value) {
    showPanel()
  }
}

// Toggle collapse state (maximize -> minimize)
const toggleCollapse = () => {
  if (isCollapsed.value) {
    // Expand: reset to default position
    isCollapsed.value = false
    isPositioned.value = false
    localStorage.removeItem('status-panel-position')
    localStorage.setItem('status-panel-collapsed', 'false')
  } else {
    // Collapse: reset to default position
    isCollapsed.value = true
    isPositioned.value = false
    localStorage.removeItem('status-panel-position')
    localStorage.setItem('status-panel-collapsed', 'true')
  }
}

// Hide panel (becomes small circle)
const hidePanel = () => {
  isVisible.value = false
  localStorage.setItem('status-panel-visible', 'false')
  // Reset to default position (bottom-right)
  isPositioned.value = false
  localStorage.removeItem('status-panel-position')
}

// Show panel (restore to maximized)
const showPanel = () => {
  isVisible.value = true
  isCollapsed.value = false
  localStorage.setItem('status-panel-visible', 'true')
  localStorage.setItem('status-panel-collapsed', 'false')

  // Always use default position when restoring (bottom-right, above input box)
  // Since position is reset when closing
  isPositioned.value = false
  localStorage.removeItem('status-panel-position')
}

const savePosition = () => {
  localStorage.setItem('status-panel-position', JSON.stringify(position.value))
  localStorage.setItem('status-panel-edge-distance', JSON.stringify(edgeDistance.value))
}

const restorePosition = () => {
  try {
    const saved = localStorage.getItem('status-panel-position')
    const savedEdge = localStorage.getItem('status-panel-edge-distance')

    if (saved && savedEdge) {
      const savedPos = JSON.parse(saved)
      const savedEdgeDist = JSON.parse(savedEdge)

      // Restore edge distance
      edgeDistance.value = savedEdgeDist

      // Calculate position from edge distance
      const width = TRIGGER_SIZE
      const height = TRIGGER_SIZE
      const topInset = getSafeTopInset()
      position.value = {
        x: Math.max(
          0,
          Math.min(window.innerWidth - savedEdgeDist.right - width, window.innerWidth - width)
        ),
        y: Math.max(
          topInset,
          Math.min(window.innerHeight - savedEdgeDist.bottom - height, window.innerHeight - height)
        )
      }
      isPositioned.value = true
    } else if (saved) {
      // Fallback to old position format (backward compatibility)
      const savedPos = JSON.parse(saved)
      const topInset = getSafeTopInset()
      position.value = {
        x: Math.max(0, Math.min(savedPos.x, window.innerWidth - TRIGGER_SIZE)),
        y: Math.max(topInset, Math.min(savedPos.y, window.innerHeight - TRIGGER_SIZE))
      }
      // Calculate edge distance from absolute position
      edgeDistance.value = {
        right: window.innerWidth - position.value.x - TRIGGER_SIZE,
        bottom: window.innerHeight - position.value.y - TRIGGER_SIZE
      }
      isPositioned.value = true
    }

    const savedCollapsed = localStorage.getItem('status-panel-collapsed')
    if (savedCollapsed !== null) {
      isCollapsed.value = savedCollapsed === 'true'
    }

    const savedVisible = localStorage.getItem('status-panel-visible')
    if (savedVisible !== null) {
      isVisible.value = savedVisible === 'true'
    }
  } catch (e) {
    console.error('Failed to restore panel state:', e)
  }
}

// Ensure panel position stays within viewport bounds and maintains edge distance
const constrainPosition = () => {
  if (!isPositioned.value) return

  // Determine current dimensions based on visibility and collapse state
  let width, height
  if (!isVisible.value) {
    // Trigger button (small circle)
    width = TRIGGER_SIZE
    height = TRIGGER_SIZE
  } else if (isCollapsed.value) {
    // Collapsed panel
    width = COLLAPSED_WIDTH
    height = COLLAPSED_HEIGHT
  } else {
    // Expanded panel
    width = PANEL_WIDTH
    height = panelRef.value?.offsetHeight || 250
  }

  // Calculate new position based on edge distance
  let newX = window.innerWidth - edgeDistance.value.right - width
  let newY = window.innerHeight - edgeDistance.value.bottom - height
  const topInset = getSafeTopInset()

  // Constrain to viewport bounds
  newX = Math.max(0, Math.min(newX, window.innerWidth - width))
  newY = Math.max(topInset, Math.min(newY, window.innerHeight - height))

  // Update position
  position.value = { x: newX, y: newY }
  savePosition()
}

onMounted(() => {
  restorePosition()

  // Ensure panel stays within viewport on initial load
  // If not positioned yet, we need to verify default position doesn't overflow
  if (!isPositioned.value) {
    const panelHeight = PANEL_HEIGHT
    const panelWidth = PANEL_WIDTH
    const defaultRight = 20
    const defaultBottom = 220

    // Calculate what left/top would be for default right/bottom positioning
    let targetLeft = window.innerWidth - panelWidth - defaultRight
    let targetTop = window.innerHeight - panelHeight - defaultBottom

    // Constrain to viewport bounds with 20px margin
    targetLeft = Math.max(20, Math.min(targetLeft, window.innerWidth - panelWidth - 20))
    targetTop = Math.max(
      getSafeTopInset(),
      Math.min(targetTop, window.innerHeight - panelHeight - 20)
    )

    // If constrained position differs from default, we need to set it explicitly
    if (
      targetLeft !== window.innerWidth - panelWidth - defaultRight ||
      targetTop !== window.innerHeight - panelHeight - defaultBottom
    ) {
      position.value = { x: targetLeft, y: targetTop }
      isPositioned.value = true
      // Calculate edge distance for this position
      edgeDistance.value = {
        right: window.innerWidth - position.value.x - panelWidth,
        bottom: window.innerHeight - position.value.y - panelHeight
      }
      savePosition()
    }
  } else {
    // Already positioned (from localStorage), ensure it's still valid
    constrainPosition()
  }

  window.addEventListener('resize', constrainPosition)
})

onUnmounted(() => {
  window.removeEventListener('resize', constrainPosition)
})

watch(
  () => workflowStore.currentWorkflowId,
  newId => {
    if (newId && isCollapsed.value) {
      isCollapsed.value = false
    }
    activeTab.value = 'main'
    childSnapshotProgress.value = new Map()
  }
)

watch(
  () => childSessionIdsFromSource.value.join('|'),
  () => {
    refreshChildSnapshots()
  },
  { immediate: true }
)

watch(
  () => hasData.value,
  (hasDataNow, hadDataBefore) => {
    // When data first appears (panel becomes visible), ensure it's within viewport
    if (hasDataNow && !hadDataBefore) {
      // Wait for DOM to render with actual content height
      nextTick(() => {
        constrainPosition()
      })
    }
  }
)
</script>

<style lang="scss" scoped>
.status-panel {
  position: fixed;
  right: 20px;
  bottom: 100px;
  width: 280px;
  background: var(--cs-bg-color);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-lg);
  box-shadow: var(--el-box-shadow-light);
  z-index: 1000;
  transition:
    box-shadow 0.2s ease,
    transform 0.1s ease;
  overflow: hidden;

  &.dragging {
    cursor: grabbing;
    box-shadow: var(--el-box-shadow-dark);
    transform: scale(1.02);
  }

  &.collapsed {
    width: auto;
    min-width: 140px;

    .panel-header {
      border-bottom: none;
      padding: 8px 12px;
    }
  }

  &:not(.dragging) {
    transition:
      left 0.3s ease,
      top 0.3s ease;
  }
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  background: var(--cs-bg-color-light);
  border-bottom: 1px solid var(--cs-border-color-light);
  cursor: grab;
  user-select: none;
  -webkit-user-select: none;
  touch-action: none;

  &:active {
    cursor: grabbing;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 8px;

    .drag-icon {
      color: var(--cs-text-color-placeholder);
      cursor: grab;
    }

    .header-title {
      font-size: var(--cs-font-size-sm);
      font-weight: 600;
      color: var(--cs-text-color-primary);
    }
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 6px;

    .collapsed-progress,
    .task-count {
      font-size: var(--cs-font-size-xs);
      color: var(--el-color-primary);
      font-weight: 600;
      padding: 2px 6px;
      background: var(--el-color-primary-light-9);
      border-radius: var(--cs-border-radius);
    }

    .action-btn {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 22px;
      height: 22px;
      border-radius: var(--cs-border-radius-round);
      cursor: pointer;
      color: var(--cs-text-color-secondary);
      transition: all 0.2s ease;

      &:hover {
        background: var(--cs-hover-bg-color);
        color: var(--cs-text-color-primary);
      }

      &.close-btn:hover {
        background: var(--el-color-danger-light-9);
        color: var(--el-color-danger);
      }
    }
  }
}

.panel-body {
  max-height: 600px;
  overflow-y: auto;
  padding: 12px;
}

.panel-tabs {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;

  .tab-btn {
    border: 1px solid var(--cs-border-color);
    background: var(--cs-bg-color-light);
    color: var(--cs-text-color-secondary);
    border-radius: var(--cs-border-radius);
    padding: 4px 10px;
    font-size: var(--cs-font-size-xs);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;

    &.active {
      color: var(--el-color-primary);
      border-color: var(--el-color-primary-light-5);
      background: var(--el-color-primary-light-9);
    }
  }

  .tab-badge {
    min-width: 16px;
    height: 16px;
    border-radius: 10px;
    background: var(--el-color-primary);
    color: #fff;
    font-size: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0 4px;
  }
}

.section {
  margin-bottom: 16px;

  &:last-child {
    margin-bottom: 0;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--cs-font-size-xs);
    color: var(--cs-text-color-secondary);
    font-weight: 500;
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 0.5px;

    .cs {
      color: var(--el-color-primary);
    }

    .section-meta {
      margin-left: auto;
      font-size: 10px;
      color: var(--cs-text-color-placeholder);
      text-transform: none;
      letter-spacing: 0;
    }
  }
}

.child-agent-list {
  list-style: none;
  padding: 0;
  margin: 0;

  .child-agent-item {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    font-size: var(--cs-font-size-xs);
    background: var(--cs-bg-color-light);
    border-radius: var(--cs-border-radius-sm);
    margin-bottom: 6px;
    border: 1px solid var(--cs-border-color);
    border-left-width: 3px;

    &.clickable {
      cursor: pointer;
      transition:
        background-color 0.2s ease,
        transform 0.2s ease;

      &:hover {
        background: var(--cs-hover-bg-color);
        transform: translateX(-1px);
      }
    }

    &:last-child {
      margin-bottom: 0;
    }

    &.running {
      border-left-color: var(--el-color-primary);
    }
    &.success {
      border-left-color: var(--el-color-success);
    }
    &.failed {
      border-left-color: var(--el-color-danger);
    }

    .child-main {
      min-width: 0;
      display: flex;
      flex-direction: column;
      gap: 6px;
      flex: 1;
    }

    .child-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      min-width: 0;
    }

    .child-agent {
      font-size: 12px;
      color: var(--cs-text-color-primary);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      font-weight: 600;
    }

    .child-status-pill {
      flex-shrink: 0;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 4px;
      min-width: 58px;
      padding: 2px 8px;
      border-radius: 999px;
      font-size: 10px;
      line-height: 1.4;
      background: var(--cs-bg-color);
      color: var(--cs-text-color-secondary);

      &.running {
        color: var(--el-color-primary);
      }

      &.success {
        color: var(--el-color-success);
      }

      &.failed {
        color: var(--el-color-danger);
      }
    }

    .child-status-pill-icon {
      flex-shrink: 0;
    }

    .child-task {
      color: var(--cs-text-color-primary);
      line-height: 1.45;
      overflow: hidden;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
    }

    .child-metrics {
      display: flex;
      align-items: flex-start;
      gap: 6px;
      min-width: 0;
    }

    .child-metric-label {
      flex-shrink: 0;
      font-size: 10px;
      color: var(--cs-text-color-placeholder);
      text-transform: uppercase;
    }

    .child-stats {
      align-items: center;
      flex-wrap: wrap;
    }

    .child-tools,
    .child-context {
      display: inline-flex;
      align-items: center;
      padding: 1px 6px;
      border-radius: 999px;
      font-family: var(--cs-font-family-mono, monospace);
      font-size: 10px;
      line-height: 1.5;
      background: var(--cs-bg-color);
      color: var(--cs-text-color-secondary);
    }

    .child-summary {
      color: var(--cs-text-color-regular);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      min-width: 0;
      flex: 1;
    }
  }
}

.progress-section {
  .progress-bar-container {
    display: flex;
    align-items: center;
    gap: 10px;

    .progress-bar {
      flex: 1;
      height: 8px;
      background: var(--cs-bg-color-light);
      border-radius: var(--cs-border-radius-xxl);
      overflow: hidden;

      .progress-fill {
        height: 100%;
        border-radius: var(--cs-border-radius-xxl);
        transition:
          width 0.3s ease,
          background-color 0.3s ease;
        background-color: var(--el-color-primary);

        // 任务进度条颜色（原来的）
        &.task-progress {
          &.start {
            background-color: var(--el-color-info);
          }

          &.medium {
            background-color: var(--el-color-primary);
          }

          &.good {
            background-color: #67c23a;
          }

          &.complete {
            background-color: var(--el-color-success);
          }
        }

        // 上下文进度条颜色（新的）
        &.context-progress {
          &.start {
            background-color: #67c23a;
          }

          &.medium {
            background-color: var(--el-color-success);
          }

          &.good {
            background-color: var(--el-color-warning);
          }

          &.complete {
            background-color: var(--el-color-danger);
          }
        }
      }
    }

    .progress-text {
      font-size: var(--cs-font-size-sm);
      font-weight: 600;
      color: var(--cs-text-color-primary);
      min-width: 36px;
      text-align: right;
    }
  }

  .usage-details {
    font-size: 10px;
    color: var(--cs-text-color-placeholder);
    margin-top: 4px;
    text-align: right;
    font-family: var(--cs-font-family-mono, monospace);
  }
}

.todo-list {
  list-style: none;
  padding: 0;
  margin: 0;

  .todo-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    font-size: var(--cs-font-size-sm);
    color: var(--cs-text-color-regular);

    .todo-icon {
      flex-shrink: 0;
      color: var(--cs-text-color-placeholder);
    }

    .todo-text {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    &.in_progress {
      color: var(--el-color-primary);
      font-weight: 500;

      .todo-icon {
        color: var(--el-color-primary);
      }
    }

    &.completed {
      color: var(--cs-text-color-secondary);

      .todo-text {
        text-decoration: line-through;
      }

      .todo-icon {
        color: var(--el-color-success);
      }
    }

    &.failed {
      color: var(--el-color-danger);

      .todo-icon {
        color: var(--el-color-danger);
      }
    }

    &.data_missing {
      color: var(--el-color-warning);

      .todo-icon {
        color: var(--el-color-warning);
      }
    }
  }
}

.more-indicator {
  text-align: center;
  font-size: var(--cs-font-size-xs);
  color: var(--cs-text-color-placeholder);
  padding-top: 4px;
  font-style: italic;

  &.clickable {
    cursor: pointer;
    transition: color 0.2s ease;

    &:hover {
      color: var(--el-color-primary);
      text-decoration: underline;
    }
  }
}

.operations-list {
  list-style: none;
  padding: 0;
  margin: 0;

  .op-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    font-size: var(--cs-font-size-xs);
    color: var(--cs-text-color-regular);
    background: var(--cs-bg-color-light);
    border-radius: var(--cs-border-radius-sm);
    margin-bottom: 4px;
    border-left: 2px solid transparent;
    transition: all 0.2s ease;

    &:last-child {
      margin-bottom: 0;
    }

    // Tool type color coding
    &.tool-file {
      border-left-color: var(--el-color-primary);

      .op-icon {
        color: var(--el-color-primary);
      }
    }

    &.tool-network {
      border-left-color: var(--el-color-success);

      .op-icon {
        color: var(--el-color-success);
      }
    }

    &.tool-system {
      border-left-color: var(--el-color-warning);

      .op-icon {
        color: var(--el-color-warning);
      }
    }

    &.tool-todo {
      border-left-color: #8b5cf6;

      .op-icon {
        color: #8b5cf6;
      }
    }

    .op-main {
      display: flex;
      align-items: center;
      gap: 6px;
      flex: 1;
      min-width: 0; // Allow flex child to shrink
    }

    .op-icon {
      flex-shrink: 0;
      color: var(--cs-text-color-secondary);
    }

    .op-name {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-family: var(--cs-font-family-mono, monospace);
    }

    .op-status {
      flex-shrink: 0;

      &.success {
        color: var(--el-color-success);
      }

      &.error {
        color: var(--el-color-danger);
      }
    }

    &.running {
      background: var(--el-color-primary-light-9);
      border: 1px solid var(--el-color-primary-light-7);

      .op-icon {
        color: var(--el-color-primary);
      }
    }

    &.error {
      background: var(--el-color-danger-light-9);
      border-left-color: var(--el-color-danger) !important;
    }
  }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px 12px;
  color: var(--cs-text-color-placeholder);
  gap: 8px;

  span {
    font-size: var(--cs-font-size-sm);
  }
}

.status-panel-trigger {
  position: fixed;
  right: 20px;
  bottom: 100px;
  width: 44px;
  height: 44px;
  background: var(--cs-bg-color);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-round);
  box-shadow: var(--el-box-shadow-light);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  z-index: 1000;
  color: var(--el-color-primary);

  .trigger-drag-area {
    position: absolute;
    inset: 0;
    cursor: grab;
  }

  &:active .trigger-drag-area {
    cursor: grabbing;
  }

  &:hover {
    box-shadow: var(--el-box-shadow-dark);
    transform: scale(1.05);
    background: var(--el-color-primary-light-9);
  }

  .trigger-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    background: var(--el-color-primary);
    color: white;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 5px;
    border-radius: 10px;
    min-width: 20px;
    text-align: center;
  }
}

.panel-body::-webkit-scrollbar {
  width: 4px;
}

.panel-body::-webkit-scrollbar-track {
  background: transparent;
}

.panel-body::-webkit-scrollbar-thumb {
  background: var(--cs-border-color);
  border-radius: 2px;
}

.panel-body::-webkit-scrollbar-thumb:hover {
  background: var(--cs-text-color-placeholder);
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}

:deep(.cs-spin) {
  animation: spin 1s linear infinite;
}
</style>
