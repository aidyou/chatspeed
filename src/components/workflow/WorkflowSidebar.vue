<template>
  <el-aside :width="sidebarWidth" :class="{ collapsed: sidebarCollapsed, dragging: isDragging }" class="sidebar"
    :style="sidebarStyle">
    <div v-show="!sidebarCollapsed" class="sidebar-tabs-container">
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
                <div
                  v-if="wf.status || getPrimaryRootName(wf)"
                  class="workflow-status-row">
                  <div class="workflow-status" v-if="wf.status">
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
        <el-tab-pane :label="$t('settings.agent.authorizedPaths')" name="files">
          <FileTree :paths="currentPaths" @add-path="$emit('add-path-from-tree', $event)"
            @remove-path="$emit('remove-path-from-tree', $event)" />
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
  }
})

const emit = defineEmits([
  'select-workflow',
  'edit-workflow',
  'delete-workflow',
  'toggle-sidebar',
  'add-path-from-tree',
  'remove-path-from-tree'
])

const activeSidebarTab = ref('history')
const searchQuery = ref('')
const hoveredWorkflowIndex = ref(null)
const selectedPrimaryRootFilter = ref('')
const runningStatuses = new Set(['thinking', 'executing', 'auditing', 'running'])
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

const getWorkflowStatusClass = (status) => {
  const normalized = String(status || '').toLowerCase()
  return runningStatuses.has(normalized) ? 'running' : normalized
}

const getWorkflowStatusLabel = (status) => {
  const normalized = String(status || '').toLowerCase()
  return runningStatuses.has(normalized) ? 'running' : status
}

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
</script>
