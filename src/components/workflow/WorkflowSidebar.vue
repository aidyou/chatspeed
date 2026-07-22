<template>
  <el-aside :width="sidebarWidth" :class="{ collapsed: sidebarCollapsed, dragging: isDragging }" class="sidebar"
    :style="sidebarStyle">
    <div v-if="sidebarCollapsed" class="sidebar-compact">
      <div class="compact-sidebar-tabs">
        <el-tooltip :content="$t('workflow.historyTab')" placement="right" :hide-after="0" :enterable="false">
          <div
            class="compact-sidebar-tab"
            :class="{ active: compactSidebarTab === 'history' }"
            @click="activeSidebarTab = 'history'">
            <cs name="skill-plan3" size="var(--cs-font-size-lg)" />
          </div>
        </el-tooltip>
        <el-tooltip :content="$t('workflow.automation.title')" placement="right" :hide-after="0" :enterable="false">
          <div
            class="compact-sidebar-tab"
            :class="{ active: compactSidebarTab === 'automation' }"
            @click="activeSidebarTab = 'automation'">
            <cs name="clock" size="var(--cs-font-size-lg)" />
          </div>
        </el-tooltip>
      </div>

      <div v-if="compactSidebarTab === 'history'" class="compact-sidebar-list compact-workflow-list">
        <div v-if="compactActiveWorkflows.length" class="compact-sidebar-group">
          <el-tooltip
            v-for="wf in compactActiveWorkflows"
            :key="wf.id"
            placement="right"
            :hide-after="0"
            :enterable="false"
            popper-class="workflow-sidebar-tooltip">
            <template #content>
              <div class="workflow-sidebar-tooltip__title">
                {{ wf.title || wf.userQuery || $t('workflow.untitled') }}
              </div>
              <div class="workflow-sidebar-tooltip__meta">
                <span class="workflow-sidebar-tooltip__status">
                  <span :class="['status-indicator', getWorkflowStatusClass(wf.status)]"></span>
                  {{ getWorkflowStatusLabel(wf.status) }}
                </span>
                <span v-if="getPrimaryRootName(wf)" class="workflow-sidebar-tooltip__root">
                  <cs name="ext-folder" />
                  {{ getPrimaryRootName(wf) }}
                </span>
              </div>
            </template>
            <div
              class="compact-sidebar-item"
              :class="[
                getWorkflowStatusClass(wf.status),
                { active: wf.id === currentWorkflowId, disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId }
              ]"
              @click="$emit('select-workflow', wf.id)">
              <span class="compact-sidebar-item__badge">
                {{ getPrimaryRootInitials(wf) }}
              </span>
              <span :class="['compact-sidebar-item__status', getWorkflowStatusClass(wf.status)]"></span>
            </div>
          </el-tooltip>
        </div>

        <div
          v-if="compactActiveWorkflows.length && compactRecentWorkflows.length"
          class="compact-sidebar-group-separator" />

        <div v-if="compactRecentWorkflows.length" class="compact-sidebar-group">
          <el-tooltip
            v-for="wf in compactRecentWorkflows"
            :key="wf.id"
            placement="right"
            :hide-after="0"
            :enterable="false"
            popper-class="workflow-sidebar-tooltip">
            <template #content>
              <div class="workflow-sidebar-tooltip__title">
                {{ wf.title || wf.userQuery || $t('workflow.untitled') }}
              </div>
              <div class="workflow-sidebar-tooltip__meta">
                <span class="workflow-sidebar-tooltip__status">
                  <span :class="['status-indicator', getWorkflowStatusClass(wf.status)]"></span>
                  {{ getWorkflowStatusLabel(wf.status) }}
                </span>
                <span v-if="getPrimaryRootName(wf)" class="workflow-sidebar-tooltip__root">
                  <cs name="ext-folder" />
                  {{ getPrimaryRootName(wf) }}
                </span>
              </div>
            </template>
            <div
              class="compact-sidebar-item"
              :class="[
                getWorkflowStatusClass(wf.status),
                { active: wf.id === currentWorkflowId, disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId }
              ]"
              @click="$emit('select-workflow', wf.id)">
              <span class="compact-sidebar-item__badge">
                {{ getPrimaryRootInitials(wf) }}
              </span>
              <span :class="['compact-sidebar-item__status', getWorkflowStatusClass(wf.status)]"></span>
            </div>
          </el-tooltip>
        </div>
      </div>

      <div v-else class="compact-sidebar-list">
        <el-tooltip
          v-for="automation in filteredAutomations"
          :key="automation.id"
          placement="right"
          :hide-after="0"
          :enterable="false"
          popper-class="workflow-sidebar-tooltip">
          <template #content>
            <div class="workflow-sidebar-tooltip__title">
              {{ automation.title || $t('workflow.automation.untitled') }}
            </div>
            <div class="workflow-sidebar-tooltip__meta">
              <span class="workflow-sidebar-tooltip__status">
                <span :class="['status-indicator', getAutomationStatusClass(automation)]"></span>
                {{ getAutomationStatusLabel(automation) }}
              </span>
              <span v-if="getPrimaryRootName(automation)" class="workflow-sidebar-tooltip__root">
                <cs name="ext-folder" />
                {{ getPrimaryRootName(automation) }}
              </span>
            </div>
          </template>
          <div
            class="compact-sidebar-item"
            :class="[
              getAutomationStatusClass(automation),
              { active: automation.id === selectedAutomationId }
            ]"
            @click="$emit('select-automation', automation.id)">
            <span class="compact-sidebar-item__badge">
              {{ getPrimaryRootInitials(automation) }}
            </span>
            <span :class="['compact-sidebar-item__status', getAutomationStatusClass(automation)]"></span>
          </div>
        </el-tooltip>
      </div>
    </div>

    <div v-else class="sidebar-tabs-container">
      <el-tabs v-model="activeSidebarTab" class="sidebar-tabs">
        <el-tab-pane :label="$t('workflow.historyTab')" name="history">
          <div class="sidebar-header">
            <el-input v-model="searchQuery" :placeholder="$t('chat.searchChat')" :clearable="true" round>
              <template #prefix>
                <cs name="search" />
              </template>
              <template #suffix>
                <el-dropdown
                  trigger="click"
                  placement="bottom-end"
                  @command="selectPrimaryRootFilter">
                  <div
                    class="workflow-root-filter-trigger"
                    :class="{ active: !!selectedPrimaryRootFilter }"
                    @click.stop>
                    <cs name="caret-down" />
                  </div>
                  <template #dropdown>
                    <el-dropdown-menu class="workflow-root-filter-menu">
                      <el-dropdown-item command="" :class="{ active: !selectedPrimaryRootFilter }">
                        <span class="dropdown-text">{{ rootFilterAllLabel }}</span>
                        <cs v-if="!selectedPrimaryRootFilter" name="check" size="14px" class="dropdown-check" />
                      </el-dropdown-item>
                      <el-dropdown-item
                        v-for="option in primaryRootOptions"
                        :key="option"
                        :command="option"
                        :class="{ active: selectedPrimaryRootFilter === option }">
                        <span class="dropdown-text">{{ option }}</span>
                        <cs v-if="selectedPrimaryRootFilter === option" name="check" size="14px" class="dropdown-check" />
                      </el-dropdown-item>
                    </el-dropdown-menu>
                  </template>
                </el-dropdown>
              </template>
            </el-input>
          </div>
          <div class="workflow-list">
            <div class="list">
              <div class="item" v-for="wf in filteredWorkflows" :key="wf.id" @click="$emit('select-workflow', wf.id)"
                @mouseenter="hoveredWorkflowIndex = wf.id" @mouseleave="hoveredWorkflowIndex = null" :class="{
                  active: wf.id === currentWorkflowId,
                  disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId
                }">
                <div class="workflow-title">{{ wf.title || wf.userQuery || $t('workflow.untitled') }}</div>
                <div class="workflow-status-row">
                  <div class="workflow-status">
                    <span :class="['status-indicator', getWorkflowStatusClass(wf.status)]"></span>
                    {{ getWorkflowStatusLabel(wf.status) }}
                  </div>
                  <div
                    v-if="getPrimaryRootName(wf)"
                    class="workflow-primary-root"
                    :title="getPrimaryRootPath(wf)">
                    {{ getPrimaryRootName(wf) }}
                  </div>
                </div>
                <div class="icons" v-show="wf.id === hoveredWorkflowIndex">
                  <div class="icon icon-edit" @click.stop="$emit('edit-workflow', wf.id)">
                    <cs name="edit" />
                  </div>
                  <div class="icon icon-delete" @click.stop="$emit('delete-workflow', wf.id)">
                    <cs name="delete" />
                  </div>
                </div>
              </div>
            </div>
          </div>
        </el-tab-pane>
        <el-tab-pane :label="$t('workflow.automation.title')" name="automation">
          <div class="sidebar-header">
            <el-input v-model="automationSearchQuery" :placeholder="$t('chat.searchChat')" :clearable="true" round>
              <template #prefix>
                <cs name="search" />
              </template>
              <template #suffix>
                <el-tooltip
                  :content="$t('workflow.automation.create')"
                  :hide-after="0"
                  :enterable="false"
                  placement="bottom">
                  <div class="workflow-root-filter-trigger" @click.stop="$emit('create-automation')">
                    <cs name="add" />
                  </div>
                </el-tooltip>
              </template>
            </el-input>
          </div>
          <div class="workflow-list">
            <div class="list">
              <div
                v-if="automations.length === 0"
                class="sidebar-empty">
                {{ $t('workflow.automation.empty') }}
              </div>
              <div
                class="item automation-item"
                v-for="automation in filteredAutomations"
                :key="automation.id"
                :class="{ active: automation.id === selectedAutomationId }"
                @click="$emit('select-automation', automation.id)"
                @mouseenter="hoveredWorkflowIndex = automation.id"
                @mouseleave="hoveredWorkflowIndex = null">
                <div class="workflow-title">
                  <cs name="clock" size="12px" />
                  {{ automation.title || $t('workflow.automation.untitled') }}
                </div>
                <div class="workflow-status-row">
                  <div class="workflow-status">
                    <span :class="['status-indicator', getAutomationStatusClass(automation)]"></span>
                    {{ getAutomationStatusLabel(automation) }}
                  </div>
                  <div
                    v-if="getPrimaryRootName(automation)"
                    class="workflow-primary-root"
                    :title="getPrimaryRootPath(automation)">
                    {{ getPrimaryRootName(automation) }}
                  </div>
                </div>
                <div class="icons" v-show="automation.id === hoveredWorkflowIndex">
                  <div class="icon icon-edit" @click.stop="$emit('edit-automation', automation.id)">
                    <cs name="edit" />
                  </div>
                  <div class="icon icon-delete" @click.stop="$emit('delete-automation', automation.id)">
                    <cs name="delete" />
                  </div>
                </div>
              </div>
            </div>
          </div>
        </el-tab-pane>
        <el-tab-pane :label="$t('settings.agent.authorizedPaths')" name="files">
          <FileTree
            :paths="currentPaths"
            @add-path="$emit('add-path-from-tree', $event)"
            @remove-path="$emit('remove-path-from-tree', $event)"
            @reference-path="$emit('insert-path-reference', $event)" />
        </el-tab-pane>
      </el-tabs>
    </div>
  </el-aside>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import FileTree from './FileTree.vue'

const { t } = useI18n()

const props = defineProps({
  workflows: {
    type: Array,
    default: () => []
  },
  currentWorkflowId: {
    type: String,
    default: null
  },
  resetPrimaryRootFilterToken: {
    type: Number,
    default: 0
  },
  sidebarCollapsed: {
    type: Boolean,
    default: false
  },
  sidebarWidth: {
    type: String,
    default: '300px'
  },
  sidebarStyle: {
    type: Object,
    default: () => ({})
  },
  currentPaths: {
    type: Array,
    default: () => []
  },
  canSwitchWorkflow: {
    type: Boolean,
    default: true
  },
  isDragging: {
    type: Boolean,
    default: false
  },
  automations: {
    type: Array,
    default: () => []
  },
  selectedAutomationId: {
    type: String,
    default: null
  },
  activeTab: {
    type: String,
    default: 'history'
  }
})

const emit = defineEmits([
  'select-workflow',
  'edit-workflow',
  'delete-workflow',
  'select-automation',
  'create-automation',
  'edit-automation',
  'delete-automation',
  'update:activeTab',
  'toggle-sidebar',
  'add-path-from-tree',
  'remove-path-from-tree',
  'insert-path-reference'
])

const activeSidebarTab = computed({
  get: () => props.activeTab,
  set: value => emit('update:activeTab', value)
})
const compactSidebarTab = computed(() =>
  activeSidebarTab.value === 'automation' ? 'automation' : 'history'
)
const searchQuery = ref('')
const automationSearchQuery = ref('')
const hoveredWorkflowIndex = ref(null)
const selectedPrimaryRootFilter = ref('')
const runningStatuses = new Set(['thinking', 'executing', 'auditing', 'running', 'stopping'])
const waitingStatuses = new Set([
  'paused',
  'awaiting_user',
  'awaiting_approval',
  'awaiting_auto_approval',
  'awaiting_sub_agent'
])
const failedStatuses = new Set(['error', 'failed'])
const cancelledStatuses = new Set(['cancelled', 'interrupted'])
const stoppedStatuses = new Set(['completed', ...failedStatuses, ...cancelledStatuses])
const workflowStatusLabels = {
  pending: 'workflow.sidebarStatus.pending',
  thinking: 'workflow.sidebarStatus.thinking',
  executing: 'workflow.sidebarStatus.executing',
  auditing: 'workflow.sidebarStatus.auditing',
  running: 'workflow.sidebarStatus.running',
  stopping: 'workflow.sidebarStatus.stopping',
  paused: 'workflow.sidebarStatus.paused',
  awaiting_user: 'workflow.sidebarStatus.awaitingUser',
  awaiting_approval: 'workflow.sidebarStatus.awaitingApproval',
  awaiting_auto_approval: 'workflow.sidebarStatus.awaitingApproval',
  awaiting_sub_agent: 'workflow.sidebarStatus.awaitingSubAgent',
  completed: 'workflow.sidebarStatus.completed',
  error: 'workflow.sidebarStatus.error',
  failed: 'workflow.sidebarStatus.failed',
  cancelled: 'workflow.sidebarStatus.cancelled',
  interrupted: 'workflow.sidebarStatus.interrupted'
}
const rootFilterAllLabel = t('common.all')

const trimTrailingSlash = (value = '') => String(value).replace(/[\\/]+$/, '')

const getWorkflowAllowedPaths = (workflow) => {
  if (Array.isArray(workflow?.allowedPaths) && workflow.allowedPaths.length) {
    return workflow.allowedPaths
  }
  if (Array.isArray(workflow?.agentConfig?.allowedPaths) && workflow.agentConfig.allowedPaths.length) {
    return workflow.agentConfig.allowedPaths
  }
  return []
}

const getPrimaryRootPath = (workflow) => {
  const [primaryRoot] = getWorkflowAllowedPaths(workflow)
  return trimTrailingSlash(primaryRoot || '')
}

const getPrimaryRootName = (workflow) => {
  const primaryRoot = getPrimaryRootPath(workflow)
  if (!primaryRoot) return ''
  const segments = primaryRoot.split(/[\\/]/).filter(Boolean)
  return segments[segments.length - 1] || primaryRoot
}

const getPathInitials = (path) => {
  const name = getPrimaryRootName({ allowedPaths: [path] })
  if (!name) return '—'

  const characters = Array.from(name.trim())
  return characters.slice(0, 2).join('').toLocaleUpperCase()
}

const getPrimaryRootInitials = (item) => getPathInitials(getPrimaryRootPath(item))

const getWorkflowStatusClass = (status) => {
  const normalized = String(status || '').toLowerCase()
  if (runningStatuses.has(normalized)) return 'running'
  if (waitingStatuses.has(normalized)) return 'waiting'
  if (failedStatuses.has(normalized)) return 'failed'
  if (cancelledStatuses.has(normalized)) return 'cancelled'
  return normalized || 'pending'
}

const getWorkflowStatusLabel = (status) => {
  const normalized = String(status || '').toLowerCase()
  const statusClass = getWorkflowStatusClass(normalized)
  const key = workflowStatusLabels[normalized] || workflowStatusLabels[statusClass]
  return t(key || workflowStatusLabels.pending)
}

const getAutomationStatusClass = (automation) =>
  automation.enabled ? 'completed' : 'paused'

const getAutomationStatusLabel = (automation) =>
  automation.enabled ? t('workflow.automation.enabled') : t('workflow.automation.disabled')

const primaryRootOptions = computed(() => {
  const seen = new Set()
  return props.workflows
    .map((workflow) => getPrimaryRootName(workflow))
    .filter((name) => {
      if (!name || seen.has(name)) return false
      seen.add(name)
      return true
    })
    .sort((a, b) => a.localeCompare(b))
})

const selectPrimaryRootFilter = (rootName) => {
  selectedPrimaryRootFilter.value = rootName || ''
}

watch(
  () => props.resetPrimaryRootFilterToken,
  () => {
    selectedPrimaryRootFilter.value = ''
  }
)

watch(
  () => [selectedPrimaryRootFilter.value, props.currentWorkflowId, props.workflows],
  () => {
    if (!selectedPrimaryRootFilter.value) return

    const currentVisible = filteredWorkflows.value.some(
      workflow => workflow.id === props.currentWorkflowId
    )
    if (currentVisible) return

    const nextWorkflowId = filteredWorkflows.value[0]?.id
    if (nextWorkflowId) {
      emit('select-workflow', nextWorkflowId)
    }
  },
  { deep: true }
)

const filteredWorkflows = computed(() => {
  return props.workflows.filter((wf) => {
    const matchesPrimaryRoot = !selectedPrimaryRootFilter.value ||
      getPrimaryRootName(wf) === selectedPrimaryRootFilter.value
    if (!matchesPrimaryRoot) return false

    if (!searchQuery.value) return true
    const query = searchQuery.value.toLowerCase()
    const title = wf.title || ''
    const userQuery = wf.userQuery || ''
    const untitled = t('workflow.untitled').toLowerCase()
    return title.toLowerCase().includes(query) ||
      userQuery.toLowerCase().includes(query) ||
      ((!title && !userQuery) && untitled.includes(query))
  })
})

const compactActiveWorkflows = computed(() =>
  filteredWorkflows.value.filter((workflow) =>
    !stoppedStatuses.has(String(workflow.status || '').toLowerCase())
  )
)

const compactRecentWorkflows = computed(() =>
  filteredWorkflows.value
    .filter((workflow) => stoppedStatuses.has(String(workflow.status || '').toLowerCase()))
    .slice(0, 5)
)

const filteredAutomations = computed(() => {
  return props.automations.filter((automation) => {
    if (!automationSearchQuery.value) return true
    const query = automationSearchQuery.value.toLowerCase()
    return String(automation.title || '').toLowerCase().includes(query)
  })
})
</script>
