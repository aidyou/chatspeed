<template>
  <Teleport to="body">
    <!-- Large panel -->
    <div
      v-if="isVisible && hasData"
      ref="panelRef"
      class="status-panel"
      :class="{ collapsed: isCollapsed, dragging: isDragging }"
      :style="panelStyle"
    >
      <!-- Drag handle/header -->
      <div
        class="panel-header"
        @mousedown="startDrag"
        @touchstart="startDrag"
      >
        <div class="header-left">
          <cs name="list" size="14px" class="drag-icon" />
          <span v-if="!isCollapsed" class="header-title">{{ t('workflow.statusPanel.title') }}</span>
        </div>
        <div class="header-actions">
          <span v-if="isCollapsed" class="collapsed-progress">{{ progressPercent }}%</span>
          <span
            v-if="!isCollapsed && todoList.length > 0"
            class="task-count"
          >
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
        <!-- Context Usage section -->
        <div class="section progress-section">
          <div class="section-header">
            <cs name="skill-piechart" size="14px" />
            <span>{{ t('workflow.statusPanel.contextUsage') || 'Context Usage' }}</span>
          </div>
          <div class="progress-bar-container">
            <div class="progress-bar">
              <div
                class="progress-fill"
                :style="{ width: `${contextUsagePercent}%` }"
                :class="contextUsageStatusClass"
              />
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
                class="progress-fill"
                :style="{ width: `${progressPercent}%` }"
                :class="progressStatusClass"
              />
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
              :class="['todo-item', item.status]"
            >
              <cs
                :name="getStatusIcon(item.status)"
                :class="{ 'cs-spin': item.status === 'in_progress' }"
                size="14px"
                class="todo-icon"
              />
              <span class="todo-text" :title="item.subject || item.title">
                {{ item.subject || item.title }}
              </span>
            </li>
          </ul>
          <div v-if="todoList.length > 10" class="more-indicator clickable" @click="isTodoExpanded = !isTodoExpanded">
            {{ isTodoExpanded ? t('common.collapse') : `+${todoList.length - 10} ${t('common.more')}` }}
          </div>
        </div>

        <!-- Recent operations section -->
        <div v-if="recentOperations.length > 0" class="section">
          <div class="section-header">
            <cs name="tool" size="14px" />
            <span>{{ t('workflow.statusPanel.recentOps') }}</span>
          </div>
          <ul class="operations-list">
            <li
              v-for="(op, index) in recentOperations"
              :key="index"
              :class="['op-item', op.status, op.toolType]"
            >
              <div class="op-main">
                <cs :name="op.icon" size="14px" class="op-icon" />
                <span class="op-name" :title="op.fullText">{{ op.name }}</span>
              </div>
              <cs
                v-if="op.status === 'running'"
                name="loading"
                size="12px"
                class="op-status cs-spin"
              />
              <cs
                v-else-if="op.status === 'success'"
                name="check"
                size="12px"
                class="op-status success"
              />
              <cs
                v-else-if="op.status === 'error'"
                name="error"
                size="12px"
                class="op-status error"
              />
            </li>
          </ul>
        </div>

        <!-- Empty state -->
        <div v-if="todoList.length === 0 && recentOperations.length === 0" class="empty-state">
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
      @click="onTriggerClick"
    >
      <div
        class="trigger-drag-area"
        @mousedown.stop.prevent="startTriggerDrag"
        @touchstart.stop.prevent="startTriggerDrag"
      ></div>
      <cs name="list" size="18px" />
      <span v-if="progressPercent > 0" class="trigger-badge">{{ progressPercent }}%</span>
    </div>
  </Teleport>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()

// Panel state
const isVisible = ref(true)
const isCollapsed = ref(false)
const isDragging = ref(false)
const isTodoExpanded = ref(false)

// Position: use left/top for unified storage
const position = ref({ x: 0, y: 0 })
const isPositioned = ref(false)

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

// Panel dimensions
const PANEL_WIDTH = 280
const PANEL_HEIGHT = 200
const COLLAPSED_WIDTH = 140
const COLLAPSED_HEIGHT = 40
const TRIGGER_SIZE = 44

// Calculate progress percentage
const progressPercent = computed(() => {
  if (todoList.value.length === 0) return 0
  const completed = todoList.value.filter(
    item => item.status === 'completed' || item.status === 'failed' || item.status === 'data_missing'
  ).length
  return Math.round((completed / todoList.value.length) * 100)
})

const completedCount = computed(() => {
  return todoList.value.filter(
    item => item.status === 'completed' || item.status === 'failed' || item.status === 'data_missing'
  ).length
})

const progressStatusClass = computed(() => {
  if (progressPercent.value === 100) return 'complete'
  if (progressPercent.value >= 60) return 'good'
  if (progressPercent.value >= 30) return 'medium'
  return 'start'
})

// Calculate Context Usage
const maxContexts = computed(() => {
  const agentId = workflowStore.currentWorkflow?.agentId
  if (!agentId) return 128000
  const agent = agentStore.agents.find(a => a.id === agentId)
  return agent?.maxContexts || 128000
})

const totalTokens = computed(() => {
  // Find the most recent message with usage information
  const lastAssistantMsg = [...messages.value]
    .reverse()
    .find(m => m.role === 'assistant' && (m.metadata?.usage || m.metadata?.tokens))
  
  if (!lastAssistantMsg) return 0
  
  const meta = lastAssistantMsg.metadata
  // 1. Try ChatMetadata style (nested tokens object)
  if (meta.tokens) {
    return meta.tokens.total || (meta.tokens.prompt + meta.tokens.completion) || 0
  }
  
  // 2. Fallback to flattened style or legacy 'usage' wrapper
  const usage = meta.usage || meta
  const input = usage.input_tokens || usage.prompt_tokens || 0
  const output = usage.output_tokens || usage.completion_tokens || 0
  return input + output
})

const contextUsagePercent = computed(() => {
  if (maxContexts.value <= 0) return 0
  const percent = Math.round((totalTokens.value / maxContexts.value) * 100)
  return Math.min(percent, 100)
})

const contextUsageStatusClass = computed(() => {
  if (contextUsagePercent.value >= 90) return 'complete'
  if (contextUsagePercent.value >= 70) return 'medium'
  if (contextUsagePercent.value >= 40) return 'good'
  return 'start'
})

const formatNumber = (num) => {
  if (!num) return '0'
  return new Intl.NumberFormat().format(num)
}

// Helper to remove <system-reminder>...</system-reminder> tags
const removeSystemReminder = (content) => {
  if (!content) return ''
  return content.replace(/<system-reminder>[\s\S]*?<\/system-reminder>/gi, '').trim()
}

const getToolInfo = (name, metadata = {}) => {
  const iconMap = {
    'read_file': { icon: 'file', toolType: 'tool-file' },
    'write_file': { icon: 'file', toolType: 'tool-file' },
    'edit_file': { icon: 'edit', toolType: 'tool-file' },
    'list_dir': { icon: 'folder', toolType: 'tool-file' },
    'glob': { icon: 'search', toolType: 'tool-file' },
    'grep': { icon: 'search', toolType: 'tool-file' },
    'web_fetch': { icon: 'link', toolType: 'tool-network' },
    'web_search': { icon: 'search', toolType: 'tool-network' },
    'bash': { icon: 'terminal', toolType: 'tool-system' },
    'todo_create': { icon: 'add', toolType: 'tool-todo' },
    'todo_update': { icon: 'check', toolType: 'tool-todo' },
    'todo_list': { icon: 'list', toolType: 'tool-todo' },
    'todo_get': { icon: 'list', toolType: 'tool-todo' },
    'finish_task': { icon: 'check-circle', toolType: 'tool-todo' }
  }

  const info = iconMap[name] || { icon: 'tool', toolType: 'tool-system' }
  
  return {
    ...info,
    shortName: metadata.title || name.replace(/_/g, ' ')
  }
}

// Calculate recent operations
const recentOperations = computed(() => {
  const toolMessages = messages.value
    .filter(m => m.role === 'tool')
    .slice(-3)
    .reverse()

  return toolMessages.map(m => {
    const meta = m.metadata || {}
    const toolCall = meta.tool_call || {}
    
    // Robustly extract name (handling both ReAct and OpenAI styles)
    const func = toolCall.function || toolCall
    const name = func.name || toolCall.name || 'Tool'

    let status = 'success'
    if (m.isError || meta.is_error) {
      status = 'error'
    } else if (isRunning.value && m === toolMessages[0]) {
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

// Hide panel when there's no data to show
const hasData = computed(() => {
  return todoList.value.length > 0 || recentOperations.value.length > 0
})

const getStatusIcon = (status) => {
  switch (status) {
    case 'completed': return 'check'
    case 'in_progress': return 'loading'
    case 'failed': return 'error'
    case 'data_missing': return 'error'
    default: return 'uncheck'
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
const startDrag = (e) => {
  if (e.target.closest('.action-btn')) return

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const rect = panelRef.value?.getBoundingClientRect()
  if (rect) {
    dragOffset.value = {
      x: clientX - rect.left,
      y: clientY - rect.top
    }
  }

  isDragging.value = true

  document.addEventListener('mousemove', onDrag)
  document.addEventListener('mouseup', stopDrag)
  document.addEventListener('touchmove', onDrag)
  document.addEventListener('touchend', stopDrag)
}

const onDrag = (e) => {
  if (!isDragging.value) return

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const newX = clientX - dragOffset.value.x
  const newY = clientY - dragOffset.value.y

  // Use different dimensions based on current state
  const width = isCollapsed.value ? COLLAPSED_WIDTH : PANEL_WIDTH
  const height = isCollapsed.value ? COLLAPSED_HEIGHT : (panelRef.value?.offsetHeight || 250)
  const bottomReserved = isCollapsed.value ? 0 : 150

  position.value = {
    x: Math.max(0, Math.min(newX, window.innerWidth - width)),
    y: Math.max(0, Math.min(newY, window.innerHeight - height - bottomReserved))
  }
}

const stopDrag = () => {
  isDragging.value = false
  document.removeEventListener('mousemove', onDrag)
  document.removeEventListener('mouseup', stopDrag)
  document.removeEventListener('touchmove', onDrag)
  document.removeEventListener('touchend', stopDrag)

  isPositioned.value = true
  savePosition()
}

// Trigger button drag
const startTriggerDrag = (e) => {
  hasDragged.value = false

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

const onTriggerDrag = (e) => {
  if (!isDragging.value) return
  e.preventDefault()

  hasDragged.value = true

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const newX = clientX - dragOffset.value.x
  const newY = clientY - dragOffset.value.y

  // Boundary limit - small circle 44x44
  position.value = {
    x: Math.max(0, Math.min(newX, window.innerWidth - TRIGGER_SIZE)),
    y: Math.max(0, Math.min(newY, window.innerHeight - TRIGGER_SIZE))
  }
}

const stopTriggerDrag = () => {
  document.removeEventListener('mousemove', onTriggerDrag)
  document.removeEventListener('mouseup', stopTriggerDrag)
  document.removeEventListener('touchmove', onTriggerDrag)
  document.removeEventListener('touchend', stopTriggerDrag)

  // Save position
  if (hasDragged.value) {
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
}

const restorePosition = () => {
  try {
    const saved = localStorage.getItem('status-panel-position')
    if (saved) {
      const savedPos = JSON.parse(saved)
      // Validate position using small circle dimensions
      position.value = {
        x: Math.max(0, Math.min(savedPos.x, window.innerWidth - TRIGGER_SIZE)),
        y: Math.max(0, Math.min(savedPos.y, window.innerHeight - TRIGGER_SIZE))
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

const onKeyDown = (e) => {
  if (e.key === 'Escape' && !isCollapsed.value) {
    isCollapsed.value = true
  }
}

onMounted(() => {
  restorePosition()
  document.addEventListener('keydown', onKeyDown)
})

onUnmounted(() => {
  document.removeEventListener('keydown', onKeyDown)
})

watch(() => workflowStore.currentWorkflowId, (newId) => {
  if (newId && isCollapsed.value) {
    isCollapsed.value = false
  }
})

watch(() => todoList.value, (newList) => {
  const hasNewInProgress = newList.some(item => item.status === 'in_progress')
  if (hasNewInProgress && isCollapsed.value) {
    isCollapsed.value = false
  }
}, { deep: true })
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
  transition: box-shadow 0.2s ease, transform 0.1s ease;
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
    transition: left 0.3s ease, top 0.3s ease;
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
  max-height: 400px;
  overflow-y: auto;
  padding: 12px;
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
        transition: width 0.3s ease, background-color 0.3s ease;
        background-color: var(--el-color-primary);

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
      .op-icon { color: var(--el-color-primary); }
    }

    &.tool-network {
      border-left-color: var(--el-color-success);
      .op-icon { color: var(--el-color-success); }
    }

    &.tool-system {
      border-left-color: var(--el-color-warning);
      .op-icon { color: var(--el-color-warning); }
    }

    &.tool-todo {
      border-left-color: #8b5cf6;
      .op-icon { color: #8b5cf6; }
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