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
                <div class="workflow-status" v-if="wf.status">
                  <span :class="['status-indicator', wf.status.toLowerCase()]"></span>
                  {{ wf.status }}
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
import { ref, computed } from 'vue'
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

defineEmits([
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

const filteredWorkflows = computed(() => {
  if (!searchQuery.value) return props.workflows
  const query = searchQuery.value.toLowerCase()
  return props.workflows.filter((wf) => {
    const title = wf.title || ''
    const userQuery = wf.userQuery || ''
    const untitled = t('workflow.untitled').toLowerCase()
    return title.toLowerCase().includes(query) ||
      userQuery.toLowerCase().includes(query) ||
      ((!title && !userQuery) && untitled.includes(query))
  })
})
</script>
