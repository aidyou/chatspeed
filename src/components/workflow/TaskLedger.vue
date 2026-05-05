<template>
  <Teleport to="body">
    <!-- 主面板 -->
    <div v-if="isVisible && (hasData || isLoading)" ref="panelRef" class="task-ledger-panel"
      :class="{ collapsed: isCollapsed, dragging: isDragging }" :style="panelStyle">
      <!-- 拖拽手柄/头部 -->
      <div class="panel-header upperLayer" @mousedown="startDrag" @touchstart="startDrag">
        <div class="header-left">
          <cs name="task" size="14px" class="drag-icon" />
          <span v-if="!isCollapsed" class="header-title">{{ t('workflow.taskLedger.title') || 'Task Ledger' }}</span>
        </div>
        <div class="header-actions">
          <!-- 进度显示 -->
          <span v-if="isCollapsed" class="collapsed-progress">{{ progressStats.percent }}%</span>
          <span v-else-if="progressStats.total > 0" class="task-count">
            {{ progressStats.finished }}/{{ progressStats.total }}
          </span>

          <!-- 状态筛选（仅展开时） -->
          <template v-if="!isCollapsed">
            <div class="filter-btn" :class="{ active: activeFilter === 'all' }" @click.stop="activeFilter = 'all'">
              {{ t('workflow.taskLedger.all') || 'All' }}
            </div>
            <div class="filter-btn" :class="{ active: activeFilter === 'pending' }" @click.stop="activeFilter = 'pending'">
              <span class="status-dot pending"></span>
            </div>
            <div class="filter-btn" :class="{ active: activeFilter === 'running' }" @click.stop="activeFilter = 'running'">
              <span class="status-dot running"></span>
            </div>
          </template>

          <div class="action-btn" @click.stop="toggleCollapse">
            <cs :name="isCollapsed ? 'fullscreen-off' : 'minimize'" size="14px" />
          </div>
          <div class="action-btn close-btn" @click.stop="hidePanel">
            <cs name="close" size="12px" />
          </div>
        </div>
      </div>

      <!-- 展开内容 -->
      <div v-if="!isCollapsed" class="panel-body">
        <!-- 进度条 -->
        <div class="progress-section" v-if="progressStats.total > 0">
          <div class="progress-bar-container">
            <div class="progress-bar">
              <div class="progress-fill" :style="{ width: `${progressStats.percent}%` }"
                :class="progressStatusClass" />
            </div>
            <span class="progress-text">{{ progressStats.percent }}%</span>
          </div>
          <div class="progress-stats">
            <span class="stat-item pending" v-if="progressStats.pending > 0">
              {{ progressStats.pending }} {{ t('workflow.taskLedger.pending') || 'pending' }}
            </span>
            <span class="stat-item running" v-if="progressStats.running > 0">
              {{ progressStats.running }} {{ t('workflow.taskLedger.running') || 'running' }}
            </span>
            <span class="stat-item completed" v-if="progressStats.completed > 0">
              {{ progressStats.completed }} {{ t('workflow.taskLedger.completed') || 'done' }}
            </span>
            <span class="stat-item failed" v-if="progressStats.failed > 0">
              {{ progressStats.failed }} {{ t('workflow.taskLedger.failed') || 'failed' }}
            </span>
          </div>
        </div>

        <!-- 任务列表 -->
        <div class="task-list" v-if="filteredTasks.length > 0">
          <div v-for="task in filteredTasks" :key="task.toolCallId" class="task-item"
            :class="[task.status, { expanded: task.isExpanded, active: selectedTaskId === task.toolCallId }]"
            @click="onTaskClick(task)">

            <!-- 任务头部 -->
            <div class="task-header">
              <div class="task-status-icon">
                <cs v-if="task.status === 'pending'" name="clock" size="12px" />
                <cs v-else-if="task.status === 'approved_running'" name="loading" size="12px" class="spinning" />
                <cs v-else-if="task.status === 'final_success'" name="check" size="12px" class="success" />
                <cs v-else-if="task.status === 'final_error'" name="error" size="12px" class="error" />
                <cs v-else-if="task.status === 'rejected'" name="close" size="12px" class="rejected" />
              </div>

              <div class="task-info">
                <div class="task-name" :title="task.title">{{ task.title }}</div>
                <div class="task-summary" :title="task.summary">{{ task.summary }}</div>
              </div>

              <div class="task-actions">
                <cs name="chevron-right" size="12px" class="expand-icon" :class="{ expanded: task.isExpanded }" />
              </div>
            </div>

            <!-- Inspector 详情面板（展开时） -->
            <div v-if="task.isExpanded" class="task-inspector" @click.stop>
              <!-- 执行信息 -->
              <div class="inspector-section">
                <div class="section-title">{{ t('workflow.taskLedger.execution') || 'Execution' }}</div>
                <div class="section-content">
                  <div class="info-row">
                    <span class="info-label">Status:</span>
                    <span class="info-value status-badge" :class="task.status">{{ formatStatus(task.status) }}</span>
                  </div>
                  <div class="info-row">
                    <span class="info-label">ID:</span>
                    <span class="info-value mono">{{ truncateId(task.toolCallId) }}</span>
                  </div>
                  <div class="info-row">
                    <span class="info-label">Updated:</span>
                    <span class="info-value">{{ formatTime(task.updatedAt) }}</span>
                  </div>
                </div>
              </div>

              <!-- 工具信息 -->
              <div class="inspector-section" v-if="task.arguments && Object.keys(task.arguments).length > 0">
                <div class="section-title">{{ t('workflow.taskLedger.arguments') || 'Arguments' }}</div>
                <div class="section-content">
                  <pre class="code-block arguments">{{ JSON.stringify(task.arguments, null, 2) }}</pre>
                </div>
              </div>

              <!-- 流式输出 -->
              <div class="inspector-section" v-if="task.streamOutput && task.streamOutput.length > 0">
                <div class="section-title">
                  {{ t('workflow.taskLedger.output') || 'Output' }}
                  <span class="line-count">({{ task.streamOutput.length }} lines)</span>
                </div>
                <div class="section-content">
                  <div class="stream-output">
                    <div v-for="(line, idx) in displayedStreamOutput(task)" :key="idx" class="stream-line">
                      {{ line }}
                    </div>
                    <div v-if="task.streamOutput.length > maxStreamLines" class="stream-more">
                      +{{ task.streamOutput.length - maxStreamLines }} more lines
                    </div>
                  </div>
                </div>
              </div>

              <!-- 结果 -->
              <div class="inspector-section" v-if="task.result">
                <div class="section-title">{{ t('workflow.taskLedger.result') || 'Result' }}</div>
                <div class="section-content">
                  <pre class="code-block result">{{ truncateResult(task.result) }}</pre>
                </div>
              </div>

              <!-- 错误信息 -->
              <div class="inspector-section" v-if="task.errorType || task.status === 'final_error'">
                <div class="section-title error">{{ t('workflow.taskLedger.error') || 'Error' }}</div>
                <div class="section-content">
                  <div class="error-message">{{ task.errorType || 'Execution failed' }}</div>
                </div>
              </div>

              <!-- 跳转到消息按钮 -->
              <div class="inspector-actions">
                <el-button size="small" text @click="scrollToMessage(task.toolCallId)">
                  <cs name="message" size="12px" />
                  {{ t('workflow.taskLedger.locateInChat') || 'Locate in Chat' }}
                </el-button>
              </div>
            </div>
          </div>
        </div>

        <!-- 空状态 -->
        <div v-else-if="!isLoading" class="empty-state">
          <cs name="task" size="32px" />
          <span>{{ t('workflow.taskLedger.empty') || 'No tasks yet' }}</span>
          <span v-if="activeFilter !== 'all'" class="empty-hint">
            {{ t('workflow.taskLedger.tryDifferentFilter') || 'Try a different filter' }}
          </span>
        </div>

        <!-- 加载状态 -->
        <div v-else class="loading-state">
          <cs name="loading" size="24px" class="spinning" />
          <span>{{ t('workflow.taskLedger.loading') || 'Loading tasks...' }}</span>
        </div>
      </div>
    </div>

    <!-- 触发按钮（小圆点） -->
    <div v-else-if="hasData || isLoading" ref="triggerRef" class="task-ledger-trigger" :style="triggerStyle"
      @click="onTriggerClick">
      <div class="trigger-drag-area" @mousedown.stop.prevent="startTriggerDrag"
        @touchstart.stop.prevent="startTriggerDrag"></div>
      <cs name="task" size="18px" />
      <span v-if="progressStats.pending > 0" class="trigger-badge pending">{{ progressStats.pending }}</span>
      <span v-else-if="progressStats.running > 0" class="trigger-badge running">
        <cs name="loading" size="10px" class="spinning" />
      </span>
    </div>
  </Teleport>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  /** 任务列表 */
  tasks: {
    type: Array,
    default: () => []
  },
  /** 进度统计 */
  progressStats: {
    type: Object,
    default: () => ({
      total: 0,
      completed: 0,
      failed: 0,
      running: 0,
      pending: 0,
      finished: 0,
      percent: 0
    })
  },
  /** 是否加载中 */
  isLoading: {
    type: Boolean,
    default: false
  },
  /** 当前 workflow ID */
  currentWorkflowId: {
    type: String,
    default: null
  }
})

const emit = defineEmits([
  'task-click',
  'toggle-expand',
  'scroll-to-message'
])

const { t } = useI18n()

// 面板状态
const isVisible = ref(true)
const isCollapsed = ref(false)
const isDragging = ref(false)
const activeFilter = ref('all')
const selectedTaskId = ref(null)
const maxStreamLines = 50

// 位置
const position = ref({ x: 0, y: 0 })
const isPositioned = ref(false)
const edgeDistance = ref({ right: 20, bottom: 220 })
const dragOffset = ref({ x: 0, y: 0 })
const hasDragged = ref(false)

// DOM 引用
const panelRef = ref(null)
const triggerRef = ref(null)

// 面板尺寸
const PANEL_WIDTH = 320
const PANEL_HEIGHT = 400
const COLLAPSED_WIDTH = 160
const COLLAPSED_HEIGHT = 40
const TRIGGER_SIZE = 44

// 计算属性
const hasData = computed(() => props.tasks.length > 0)

const progressStatusClass = computed(() => {
  const pct = props.progressStats.percent
  if (pct === 100) return 'complete'
  if (pct >= 60) return 'good'
  if (pct >= 30) return 'medium'
  return 'start'
})

const filteredTasks = computed(() => {
  let tasks = props.tasks
  if (activeFilter.value === 'pending') {
    tasks = tasks.filter(t => t.status === 'pending')
  } else if (activeFilter.value === 'running') {
    tasks = tasks.filter(t => t.status === 'approved_running')
  }
  return tasks
})

const panelStyle = computed(() => {
  if (!isPositioned.value) {
    return { right: '20px', bottom: '220px', left: 'auto', top: 'auto' }
  }
  return { left: `${position.value.x}px`, top: `${position.value.y}px`, right: 'auto', bottom: 'auto' }
})

const triggerStyle = computed(() => {
  if (!isPositioned.value) {
    return { right: '20px', bottom: '220px', left: 'auto', top: 'auto' }
  }
  return { left: `${position.value.x}px`, top: `${position.value.y}px`, right: 'auto', bottom: 'auto' }
})

// 方法
const formatStatus = (status) => {
  const map = {
    'pending': t('workflow.taskLedger.status.pending') || 'Pending',
    'approved_running': t('workflow.taskLedger.status.running') || 'Running',
    'rejected': t('workflow.taskLedger.status.rejected') || 'Rejected',
    'final_success': t('workflow.taskLedger.status.success') || 'Success',
    'final_error': t('workflow.taskLedger.status.error') || 'Error'
  }
  return map[status] || status
}

const truncateId = (id) => {
  if (!id) return ''
  if (id.length <= 12) return id
  return id.substring(0, 6) + '...' + id.substring(id.length - 4)
}

const truncateResult = (result) => {
  if (!result) return ''
  if (result.length <= 500) return result
  return result.substring(0, 500) + '\n... (truncated)'
}

const formatTime = (timestamp) => {
  if (!timestamp) return '-'
  const date = new Date(timestamp)
  return date.toLocaleTimeString()
}

const displayedStreamOutput = (task) => {
  if (!task.streamOutput) return []
  return task.streamOutput.slice(-maxStreamLines)
}

const onTaskClick = (task) => {
  selectedTaskId.value = task.toolCallId
  emit('task-click', task)
  emit('toggle-expand', task.toolCallId)
}

const scrollToMessage = (toolCallId) => {
  emit('scroll-to-message', toolCallId)
}

// 拖拽处理
const startDrag = (e) => {
  if (e.target.closest('.action-btn') || e.target.closest('.filter-btn')) return

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const rect = panelRef.value?.getBoundingClientRect()
  if (rect) {
    dragOffset.value = { x: clientX - rect.left, y: clientY - rect.top }
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

  const width = isCollapsed.value ? COLLAPSED_WIDTH : PANEL_WIDTH
  const height = isCollapsed.value ? COLLAPSED_HEIGHT : (panelRef.value?.offsetHeight || 400)
  const bottomReserved = 150

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

  const width = isCollapsed.value ? COLLAPSED_WIDTH : PANEL_WIDTH
  const height = isCollapsed.value ? COLLAPSED_HEIGHT : (panelRef.value?.offsetHeight || 400)
  edgeDistance.value = {
    right: window.innerWidth - position.value.x - width,
    bottom: window.innerHeight - position.value.y - height
  }

  savePosition()
}

const startTriggerDrag = (e) => {
  hasDragged.value = false

  const clientX = e.type.includes('touch') ? e.touches[0].clientX : e.clientX
  const clientY = e.type.includes('touch') ? e.touches[0].clientY : e.clientY

  const rect = triggerRef.value?.getBoundingClientRect()
  if (rect) {
    if (!isPositioned.value) {
      position.value = { x: rect.left, y: rect.top }
      isPositioned.value = true
    }
    dragOffset.value = { x: clientX - rect.left, y: clientY - rect.top }
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

  if (hasDragged.value) {
    edgeDistance.value = {
      right: window.innerWidth - position.value.x - TRIGGER_SIZE,
      bottom: window.innerHeight - position.value.y - TRIGGER_SIZE
    }
    savePosition()
  }

  isDragging.value = false
}

const onTriggerClick = () => {
  if (!hasDragged.value) {
    showPanel()
  }
}

const toggleCollapse = () => {
  isCollapsed.value = !isCollapsed.value
  isPositioned.value = false
  localStorage.setItem('task-ledger-collapsed', String(isCollapsed.value))
}

const hidePanel = () => {
  isVisible.value = false
  localStorage.setItem('task-ledger-visible', 'false')
}

const showPanel = () => {
  isVisible.value = true
  isCollapsed.value = false
  isPositioned.value = false
  localStorage.setItem('task-ledger-visible', 'true')
  localStorage.setItem('task-ledger-collapsed', 'false')
}

const savePosition = () => {
  localStorage.setItem('task-ledger-position', JSON.stringify(position.value))
  localStorage.setItem('task-ledger-edge-distance', JSON.stringify(edgeDistance.value))
}

const restorePosition = () => {
  try {
    const saved = localStorage.getItem('task-ledger-position')
    const savedEdge = localStorage.getItem('task-ledger-edge-distance')

    if (saved && savedEdge) {
      const savedPos = JSON.parse(saved)
      const savedEdgeDist = JSON.parse(savedEdge)

      edgeDistance.value = savedEdgeDist

      const width = TRIGGER_SIZE
      const height = TRIGGER_SIZE
      position.value = {
        x: Math.max(0, Math.min(window.innerWidth - savedEdgeDist.right - width, window.innerWidth - width)),
        y: Math.max(0, Math.min(window.innerHeight - savedEdgeDist.bottom - height, window.innerHeight - height))
      }
      isPositioned.value = true
    }

    const savedCollapsed = localStorage.getItem('task-ledger-collapsed')
    if (savedCollapsed !== null) {
      isCollapsed.value = savedCollapsed === 'true'
    }

    const savedVisible = localStorage.getItem('task-ledger-visible')
    if (savedVisible !== null) {
      isVisible.value = savedVisible === 'true'
    }
  } catch (e) {
    console.error('Failed to restore task ledger position:', e)
  }
}

const constrainPosition = () => {
  if (!isPositioned.value) return

  let width, height
  if (!isVisible.value) {
    width = TRIGGER_SIZE
    height = TRIGGER_SIZE
  } else if (isCollapsed.value) {
    width = COLLAPSED_WIDTH
    height = COLLAPSED_HEIGHT
  } else {
    width = PANEL_WIDTH
    height = panelRef.value?.offsetHeight || 400
  }

  let newX = window.innerWidth - edgeDistance.value.right - width
  let newY = window.innerHeight - edgeDistance.value.bottom - height

  newX = Math.max(0, Math.min(newX, window.innerWidth - width))
  newY = Math.max(0, Math.min(newY, window.innerHeight - height))

  position.value = { x: newX, y: newY }
  savePosition()
}

// 监听 workflow 切换
watch(() => props.currentWorkflowId, (newId) => {
  if (newId && isCollapsed.value) {
    isCollapsed.value = false
  }
  // 重置选中状态
  selectedTaskId.value = null
})

// 监听数据变化，自动展开面板
watch(() => props.tasks.length, (newLen, oldLen) => {
  if (newLen > 0 && oldLen === 0 && !isVisible.value) {
    showPanel()
  }
})

onMounted(() => {
  restorePosition()
  window.addEventListener('resize', constrainPosition)
})

onUnmounted(() => {
  window.removeEventListener('resize', constrainPosition)
})

// 暴露方法
defineExpose({
  showPanel,
  hidePanel,
  toggleCollapse
})
</script>

<style lang="scss" scoped>
.task-ledger-panel {
  position: fixed;
  right: 20px;
  bottom: 220px;
  width: 320px;
  background: var(--cs-bg-color);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-lg);
  box-shadow: var(--el-box-shadow-light);
  z-index: 1000;
  transition: box-shadow 0.2s ease, transform 0.1s ease;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  max-height: 70vh;

  &.dragging {
    cursor: grabbing;
    box-shadow: var(--el-box-shadow-dark);
    transform: scale(1.02);
  }

  &.collapsed {
    width: auto;
    min-width: 160px;

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
  flex-shrink: 0;

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

    .filter-btn {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 24px;
      height: 22px;
      border-radius: var(--cs-border-radius);
      cursor: pointer;
      background: var(--cs-bg-color);
      border: 1px solid transparent;
      transition: all 0.2s ease;

      &:hover {
        background: var(--cs-hover-bg-color);
      }

      &.active {
        background: var(--el-color-primary-light-9);
        border-color: var(--el-color-primary);
      }

      .status-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;

        &.pending { background: var(--el-color-warning); }
        &.running { background: var(--el-color-primary); }
      }
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
  overflow-y: auto;
  padding: 12px;
  flex: 1;
  min-height: 0;
}

.progress-section {
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--cs-border-color-light);

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

        &.start { background-color: var(--el-color-info); }
        &.medium { background-color: var(--el-color-primary); }
        &.good { background-color: #67c23a; }
        &.complete { background-color: var(--el-color-success); }
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

  .progress-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;

    .stat-item {
      font-size: 11px;
      padding: 2px 8px;
      border-radius: var(--cs-border-radius);

      &.pending { background: var(--el-color-warning-light-9); color: var(--el-color-warning); }
      &.running { background: var(--el-color-primary-light-9); color: var(--el-color-primary); }
      &.completed { background: var(--el-color-success-light-9); color: var(--el-color-success); }
      &.failed { background: var(--el-color-danger-light-9); color: var(--el-color-danger); }
    }
  }
}

.task-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.task-item {
  background: var(--cs-bg-color-light);
  border-radius: var(--cs-border-radius);
  border: 1px solid transparent;
  cursor: pointer;
  transition: all 0.2s ease;
  overflow: hidden;

  &:hover {
    border-color: var(--cs-border-color);
    background: var(--cs-hover-bg-color);
  }

  &.active {
    border-color: var(--el-color-primary);
  }

  &.expanded {
    border-color: var(--el-color-primary-light-7);
  }

  // 状态样式
  &.pending { border-left: 3px solid var(--el-color-warning); }
  &.approved_running { border-left: 3px solid var(--el-color-primary); }
  &.final_success { border-left: 3px solid var(--el-color-success); }
  &.final_error { border-left: 3px solid var(--el-color-danger); }
  &.rejected { border-left: 3px solid var(--cs-text-color-placeholder); }

  .task-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;

    .task-status-icon {
      flex-shrink: 0;
      width: 20px;
      height: 20px;
      display: flex;
      align-items: center;
      justify-content: center;

      .success { color: var(--el-color-success); }
      .error { color: var(--el-color-danger); }
      .rejected { color: var(--cs-text-color-placeholder); }

      .spinning {
        animation: spin 1s linear infinite;
      }
    }

    .task-info {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
      gap: 2px;

      .task-name {
        font-size: var(--cs-font-size-sm);
        font-weight: 500;
        color: var(--cs-text-color-primary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .task-summary {
        font-size: var(--cs-font-size-xs);
        color: var(--cs-text-color-secondary);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
    }

    .task-actions {
      flex-shrink: 0;

      .expand-icon {
        color: var(--cs-text-color-placeholder);
        transition: transform 0.2s ease;

        &.expanded {
          transform: rotate(90deg);
        }
      }
    }
  }
}

.task-inspector {
  padding: 12px;
  padding-top: 0;
  border-top: 1px solid var(--cs-border-color-light);
  background: var(--cs-bg-color);

  .inspector-section {
    margin-bottom: 12px;

    &:last-child {
      margin-bottom: 0;
    }

    .section-title {
      font-size: 10px;
      font-weight: 600;
      color: var(--cs-text-color-secondary);
      text-transform: uppercase;
      letter-spacing: 0.5px;
      margin-bottom: 6px;

      &.error {
        color: var(--el-color-danger);
      }

      .line-count {
        font-weight: normal;
        color: var(--cs-text-color-placeholder);
        margin-left: 4px;
      }
    }

    .section-content {
      .info-row {
        display: flex;
        gap: 8px;
        font-size: var(--cs-font-size-xs);
        margin-bottom: 4px;

        .info-label {
          color: var(--cs-text-color-secondary);
          min-width: 60px;
        }

        .info-value {
          color: var(--cs-text-color-regular);
          word-break: break-all;

          &.mono {
            font-family: var(--cs-font-family-mono, monospace);
          }

          &.status-badge {
            padding: 1px 6px;
            border-radius: var(--cs-border-radius);
            font-size: 10px;
            font-weight: 500;

            &.pending { background: var(--el-color-warning-light-9); color: var(--el-color-warning); }
            &.approved_running { background: var(--el-color-primary-light-9); color: var(--el-color-primary); }
            &.final_success { background: var(--el-color-success-light-9); color: var(--el-color-success); }
            &.final_error { background: var(--el-color-danger-light-9); color: var(--el-color-danger); }
            &.rejected { background: var(--cs-bg-color-light); color: var(--cs-text-color-placeholder); }
          }
        }
      }

      .code-block {
        background: var(--cs-bg-color-light);
        border-radius: var(--cs-border-radius-sm);
        padding: 8px;
        font-size: 11px;
        font-family: var(--cs-font-family-mono, monospace);
        overflow-x: auto;
        max-height: 200px;
        overflow-y: auto;
        margin: 0;

        &.arguments {
          color: var(--cs-text-color-regular);
        }

        &.result {
          color: var(--el-color-success);
        }
      }

      .stream-output {
        background: var(--cs-bg-color-light);
        border-radius: var(--cs-border-radius-sm);
        padding: 8px;
        font-size: 11px;
        font-family: var(--cs-font-family-mono, monospace);
        max-height: 200px;
        overflow-y: auto;

        .stream-line {
          padding: 1px 0;
          border-bottom: 1px solid var(--cs-border-color-light);
          white-space: pre-wrap;
          word-break: break-all;

          &:last-child {
            border-bottom: none;
          }
        }

        .stream-more {
          color: var(--cs-text-color-placeholder);
          font-style: italic;
          padding-top: 4px;
          text-align: center;
        }
      }

      .error-message {
        color: var(--el-color-danger);
        font-size: var(--cs-font-size-xs);
        padding: 8px;
        background: var(--el-color-danger-light-9);
        border-radius: var(--cs-border-radius-sm);
      }
    }
  }

  .inspector-actions {
    display: flex;
    justify-content: flex-end;
    padding-top: 8px;
    border-top: 1px solid var(--cs-border-color-light);
  }
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px 12px;
  color: var(--cs-text-color-placeholder);
  gap: 8px;

  span {
    font-size: var(--cs-font-size-sm);
  }

  .empty-hint {
    font-size: var(--cs-font-size-xs);
    font-style: italic;
  }
}

.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px 12px;
  color: var(--cs-text-color-placeholder);
  gap: 12px;

  .spinning {
    animation: spin 1s linear infinite;
  }
}

.task-ledger-trigger {
  position: fixed;
  right: 20px;
  bottom: 220px;
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
    font-size: 10px;
    font-weight: 600;
    padding: 2px 5px;
    border-radius: 10px;
    min-width: 20px;
    text-align: center;

    &.pending {
      background: var(--el-color-warning);
      color: white;
    }

    &.running {
      background: var(--el-color-primary);
      color: white;
      width: 20px;
      height: 20px;
      display: flex;
      align-items: center;
      justify-content: center;

      .spinning {
        animation: spin 1s linear infinite;
      }
    }
  }
}

// 滚动条样式
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
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
</style>
