<template>
  <div class="workflow-layout">
    <Titlebar :show-menu-button="settingStore.settings.showMenuButton">
      <template #left>
        <el-tooltip :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)" placement="right"
          :hide-after="0" :enterable="false">
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
        </el-tooltip>
      </template>
      <template #center>
        <div v-if="displayAllowedPathTitle" class="workflow-titlebar-primary-path" :title="displayAllowedPathTitle">
          {{ displayAllowedPathTitle }}
        </div>
      </template>
      <template #right>
        <el-dropdown v-if="pendingApprovalList.length > 0" trigger="click" @command="handleApprovalCommand">
          <div class="icon-btn upperLayer approval-queue-btn blinking">
            <cs name="approval" />
            <span class="approval-queue-count">{{ approvalQueueCount }}</span>
          </div>
          <template #dropdown>
            <el-dropdown-menu class="approval-queue-menu">
              <el-dropdown-item
                v-for="item in pendingApprovalList"
                :key="item.key"
                :command="item.sessionId">
                <div class="approval-menu-item">
                  <div class="approval-menu-workflow">{{ item.workflowTitle }}</div>
                  <div class="approval-menu-action">{{ item.action }}</div>
                </div>
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <div class="icon-btn upperLayer" @click="toggleWorkflowApprovalMute">
          <el-tooltip :content="$t(`workflow.${workflowApprovalMuted ? 'unmuteApprovalNotifications' : 'muteApprovalNotifications'}`)" :hide-after="0" :enterable="false"
            placement="bottom">
            <cs :name="workflowApprovalMuted ? 'mute' : 'unmute'" />
          </el-tooltip>
        </div>
        <div class="icon-btn upperLayer pin-btn" @click="onPin" :class="{ active: isAlwaysOnTop }">
          <el-tooltip :content="$t(`common.${isAlwaysOnTop ? 'unpin' : 'pin'}`)" :hide-after="0" :enterable="false"
            placement="bottom">
            <cs name="pin" />
          </el-tooltip>
        </div>
      </template>
    </Titlebar>

    <div class="workflow-main">
      <WorkflowSidebar :workflows="filteredWorkflows" :current-workflow-id="currentWorkflowId"
        :sidebar-collapsed="sidebarCollapsed" :sidebar-width="sidebarWidth" :sidebar-style="sidebarStyle"
        :current-paths="currentPaths" :can-switch-workflow="canSwitchWorkflow" :is-dragging="isDragging"
        @select-workflow="selectWorkflow" @edit-workflow="onEditWorkflow" @delete-workflow="onDeleteWorkflow"
        @add-path-from-tree="onAddPathFromTree" @remove-path-from-tree="onRemovePathFromTree" />

      <!-- Resize Handle -->
      <div v-if="!sidebarCollapsed" class="sidebar-resize-handle" :class="{ dragging: isDragging }"
        @mousedown="onResizeStart" />

      <!-- Main container -->
      <el-container class="main-container">
        <WorkflowMessageList ref="messageListRef" :messages="enhancedMessages" :is-running="isRunning"
          :queued-messages="workflowStore.messageQueue"
          :is-chatting="isChatting" :chat-state="chatState" :is-compressing="isCompressing"
          :compression-message="compressionMessage" :last-assistant-message="lastAssistantMessage"
          :approval-loading="approvalLoading" :active-approval-id="activeApprovalId"
          :ask-user-submitting="askUserSubmitting"
          :is-message-expanded="isMessageExpanded" :is-reasoning-expanded="isReasoningExpanded"
          :remove-system-reminder="removeSystemReminder" :get-diff-markdown="getDiffMarkdown"
          :parse-choice-content="parseChoiceContent" :get-parsed-message="getParsedMessage"
          :get-reasoning-preview="getReasoningPreview" :should-show-tool-raw-content="shouldShowToolRawContent"
          :pending-count="currentWorkflowPendingApprovals.length"
          @toggle-expand="toggleMessageExpand"
          @toggle-reasoning="toggleReasoningExpand"
          @submit-ask-user="submitAskUserResponse"
          @approve-tool="onApproveAction" @approve-all-tool="onApproveAllAction"
          @approve-all-pending="onApproveAllPendingAction"
          @reject-tool="onRejectAction" />

      <!-- Status Panel (Floating) -->
      <StatusPanel />

        <!-- Input Area -->
        <WorkflowInputArea ref="inputAreaRef" v-model:input-message="inputMessage" :is-running="isRunning"
          :has-live-session="hasLiveSession" :wait-reason="waitReason"
          :current-workflow="currentWorkflow"
          :current-workflow-id="currentWorkflowId" :selected-agent="selectedAgent" :can-edit-agent="canEditCurrentWorkflowAgent"
          :show-planning-mode-toggle="showPlanningModeToggle"
          :active-model-name="activeModelName"
          :planning-mode="planningMode" :approval-level="approvalLevel" :final-audit-mode="finalAuditMode"
          :agents="agentStore.agents" :show-skill-suggestions="showSkillSuggestions"
          :show-file-suggestions="showFileSuggestions" :filtered-system-skills="filteredSystemSkills"
          :file-suggestions="fileSuggestions" :selected-skill-index="selectedSkillIndex"
          :selected-file-index="selectedFileIndex" :on-input-key-down="onInputKeyDown"
          :on-composition-start="onCompositionStart" :on-composition-end="onCompositionEnd"
          :on-skill-select="onSkillSelect" :on-file-select="onFileSelect" @send-message="onSendMessage"
          @continue="onContinue" @stop="onStop" @approve-plan="onApprovePlan"
          @toggle-planning-mode="planningMode = !planningMode" @toggle-final-audit-mode="toggleFinalAuditMode"
          @update-approval-level="approvalLevel = $event" @update-selected-agent="onSelectedAgentChange"
          @create-new-workflow="createNewWorkflow"
          @open-model-selector="openModelSelector" />
      </el-container>
    </div>

    <!-- Edit workflow dialog -->
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

    <WorkflowModelSelector v-model="modelSelectorVisible" :initial-tab="modelSelectorTab" :agent="selectedAgent"
      @save="onModelConfigSave" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { invokeWrapper } from '@/libs/tauri'

import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'

import Titlebar from '@/components/window/Titlebar.vue'
import StatusPanel from '@/components/workflow/StatusPanel.vue'
import WorkflowModelSelector from '@/components/workflow/WorkflowModelSelector.vue'
import WorkflowSidebar from '@/components/workflow/WorkflowSidebar.vue'
import WorkflowMessageList from '@/components/workflow/WorkflowMessageList.vue'
import WorkflowInputArea from '@/components/workflow/WorkflowInputArea.vue'

// Composables
import { useWorkflowSidebar } from '@/composables/workflow/useWorkflowSidebar'
import { useWorkflowChat } from '@/composables/workflow/useWorkflowChat'
import { useWorkflowMessages } from '@/composables/workflow/useWorkflowMessages'
import { useWorkflowApproval } from '@/composables/workflow/useWorkflowApproval'
import { useWorkflowPaths } from '@/composables/workflow/useWorkflowPaths'
import { useWorkflowInput } from '@/composables/workflow/useWorkflowInput'
import { useWorkflowCore } from '@/composables/workflow/useWorkflowCore'
import { TERMINAL_STATUSES } from '@/composables/workflow/signalTypes'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

  // Component refs
  const messageListRef = ref(null)
  const inputAreaRef = ref(null)

// Unlisten refs
const unlistenFocusInput = ref(null)

// OS type
const osType = ref('')

// ============================================================
// Local state - MUST be defined FIRST before any composables
// ============================================================
const selectedAgent = ref(null)
const approvalLevel = ref('default')
const finalAuditMode = ref('off')
const planningMode = ref(false)

const showPlanningModeToggle = computed(() => {
  const workflow = workflowStore.currentWorkflow
  if (!workflow) return true

  const hasStartedContent = Boolean(String(workflow.userQuery || '').trim()) || (workflow.messagesCount || 0) > 0
  const status = String(workflow.status || '').toLowerCase()
  return !workflowStore.hasLiveSession && (!hasStartedContent || TERMINAL_STATUSES.includes(status))
})

// System skills
const systemSkills = ref([])
const fetchSystemSkills = async () => {
  try {
    const result = await invokeWrapper('get_system_skills')
    systemSkills.value = result || []
  } catch (error) {
    console.error('Failed to fetch system skills:', error)
  }
}

// ============================================================
// Composables with NO dependencies on local state
// ============================================================

// Sidebar composable
const {
  sidebarCollapsed,
  sidebarWidth,
  sidebarStyle,
  isDragging,
  onToggleSidebar,
  onResizeStart,
  updateMaxWidth
} = useWorkflowSidebar()

// Chat/Streaming composable
const {
  chattingParser,
  chatState,
  isChatting,
  isCompressing,
  compressionMessage,
  clearRetryTimer,
  getReasoningPreview,
  resetChatState,
  setRetryStatus,
  processChunk,
  processReasoningChunk,
  setCompressionStatus
} = useWorkflowChat()

// Messages composable
const {
  expandedMessages,
  expandedReasonings,
  enhancedMessages,
  lastAssistantMessage,
  toggleMessageExpand,
  isMessageExpanded,
  toggleReasoningExpand,
  isReasoningExpanded,
  removeSystemReminder,
  getDiffMarkdown,
  parseChoiceContent,
  getParsedMessage,
  shouldShowToolRawContent
} = useWorkflowMessages()

// ============================================================
// Composables that DEPEND on local state
// ============================================================

// Paths composable - needs selectedAgent
const {
  pendingPaths,
  currentPaths,
  canEditPaths,
  displayAllowedPath,
  onAddPathFromTree,
  onRemovePathFromTree
} = useWorkflowPaths({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId),
  selectedAgent: computed(() => selectedAgent.value)
})

// Input composable - needs currentPaths, systemSkills
const inputComposable = useWorkflowInput({
  inputRef: computed(() => inputAreaRef.value?.inputRef),
  onSendMessage: null, // Will be set after core composable is initialized
  currentPaths: computed(() => currentPaths.value),
  systemSkills: computed(() => systemSkills.value)
})

const {
  inputMessage,
  showSkillSuggestions,
  showFileSuggestions,
  selectedSkillIndex,
  selectedFileIndex,
  fileSuggestions,
  filteredSystemSkills,
  onInputKeyDown,
  onCompositionStart,
  onCompositionEnd,
  onSkillSelect: originalOnSkillSelect,
  onFileSelect,
  clearInput
} = inputComposable

// Core workflow composable - needs all of the above
const core = useWorkflowCore({
  selectedAgent,
  planningMode,
  approvalLevel,
  finalAuditMode,
  pendingPaths,
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId),
  currentWorkflow: computed(() => workflowStore.currentWorkflow),
  chattingParser,
  chatState,
  enhancedMessages,
  isCompressing,
  compressionMessage,
  fetchSystemSkills,
  resetChatState,
  clearRetryTimer,
  setRetryStatus,
  processChunk,
  processReasoningChunk,
  setCompressionStatus,
  scrollToBottom: (force = false) => messageListRef.value?.scrollToBottom(force)
})

const {
  unlistenWorkflowEvents,
  modelSelectorVisible,
  modelSelectorTab,
  editWorkflowDialogVisible,
  editWorkflowId,
  editWorkflowTitle,
  workflows,
  isRunning,
  hasLiveSession,
  waitReason,
  canStop,
  canContinue,
  activeModelName,
  pendingApprovalList,
  getPendingApprovalEntry,
  clearPendingApprovalEntry,
  upsertPendingApprovalEntry,
  canSwitchWorkflow,
  selectWorkflow,
  startNewWorkflow,
  onSendMessage: coreOnSendMessage,
  handleBuiltinCommand,
  onContinue,
  onApprovePlan,
  onStop,
  openModelSelector,
  onModelConfigSave,
  onEditWorkflow,
  onSaveEditWorkflow,
  onDeleteWorkflow,
  createNewWorkflow: coreCreateNewWorkflow,
  toggleFinalAuditMode
} = core

// Approval composable
const {
  approvalLoading,
  activeApprovalId,
  onApproveAction,
  onApproveAllAction,
  onRejectAction
} = useWorkflowApproval({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId),
  getPendingApprovalEntry,
  clearPendingApprovalEntry,
  upsertPendingApprovalEntry
})

// Set up the onSendMessage callback for the input composable
inputComposable.onSendMessage.value = async () => {
  const message = inputMessage.value
  if (!message.trim()) return

  clearInput()
  const wasCommand = await coreOnSendMessage(message)
  return wasCommand
}

// ============================================================
// Wrapper functions combining multiple composables
// ============================================================

// Wrapper function that calls the input composable's send handler
const onSendMessage = async () => {
  if (inputComposable.onSendMessage.value) {
    return await inputComposable.onSendMessage.value()
  }
}

// Wrapper for createNewWorkflow that also clears input
const createNewWorkflow = async () => {
  await coreCreateNewWorkflow()
  clearInput()
}

// Wrapper for skill select that properly handles send
const onSkillSelect = (skill) => {
  originalOnSkillSelect(skill)
  // If it was a command (UI action), the input now contains the command
  // We need to trigger send manually since originalOnSkillSelect doesn't have access to onSendMessage
  if (skill.type === 'command') {
    onSendMessage()
  }
}

// Approve all pending approval items for the current workflow by sending individual approve signals for each
const onApproveAllPendingAction = () => {
  for (const entry of currentWorkflowPendingApprovals.value) {
    onApproveAction(entry.id, entry.sessionId)
  }
}

// ============================================================
// Computed properties
// ============================================================

const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)
const workflowApprovalMuted = computed(() => !!settingStore.settings.workflowApprovalMuted)
const approvalQueueCount = computed(() => {
  const count = pendingApprovalList.value.length
  return count > 9 ? '9+' : String(count)
})

// Only count and approve entries for the current workflow
const currentWorkflowPendingApprovals = computed(() =>
  pendingApprovalList.value.filter(
    entry => entry.sessionId === currentWorkflowId.value
  )
)

const displayAllowedPathTitle = computed(() => {
  if (!currentPaths.value?.length) return ''
  return displayAllowedPath.value || ''
})

const getWorkflowSortTime = (workflow) => {
  const candidates = [
    workflow?.updatedAtMs,
    workflow?.updated_at_ms,
    workflow?.updatedAt,
    workflow?.updated_at,
    workflow?.createdAt,
    workflow?.created_at
  ]

  for (const value of candidates) {
    if (typeof value === 'number' && Number.isFinite(value)) {
      return value
    }
    if (typeof value === 'string' && value) {
      const timestamp = Date.parse(value)
      if (!Number.isNaN(timestamp)) {
        return timestamp
      }
    }
  }

  return 0
}

const filteredWorkflows = computed(() => {
  const searchQuery = '' // From WorkflowSidebar component
  const base = !searchQuery
    ? workflows.value
    : workflows.value.filter((wf) =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.toLowerCase())
    )

  return [...base].sort((a, b) => getWorkflowSortTime(b) - getWorkflowSortTime(a))
})

const askUserSubmitting = ref(false)

const canEditCurrentWorkflowAgent = computed(() => {
  if (!currentWorkflowId.value || !currentWorkflow.value) {
    return true
  }

  const hasQuery = !!currentWorkflow.value.userQuery?.trim()
  const hasMessages = workflowStore.messages.length > 0
  return !hasLiveSession.value && !hasQuery && !hasMessages
})

const onSelectedAgentChange = async (agent) => {
  selectedAgent.value = agent

  if (!currentWorkflowId.value || !canEditCurrentWorkflowAgent.value || !agent) {
    return
  }

  try {
    const agentConfigResult = await invokeWrapper('update_workflow_agent_id', {
      sessionId: currentWorkflowId.value,
      agentId: agent.id
    })
    const agentConfig = typeof agentConfigResult === 'string'
      ? JSON.parse(agentConfigResult)
      : agentConfigResult

    if (workflowStore.currentWorkflow) {
      workflowStore.currentWorkflow.agentId = agent.id
      workflowStore.currentWorkflow.agentConfig = agentConfig || {}
      workflowStore.currentWorkflow.allowedPaths = agentConfig?.allowedPaths || []
      workflowStore.currentWorkflow.shellPolicy = agentConfig?.shellPolicy || []
      workflowStore.setShellPolicy(agentConfig?.shellPolicy || [])
      workflowStore.setAutoApprovedTools(agentConfig?.autoApprove || [])
    }

    if (agentConfig?.approvalLevel) {
      approvalLevel.value = agentConfig.approvalLevel
    }
    if (agentConfig?.finalAudit !== undefined && agentConfig?.finalAudit !== null) {
      finalAuditMode.value = agentConfig.finalAudit ? 'on' : 'off'
    }
    if (agentConfig?.phase) {
      planningMode.value = String(agentConfig.phase).toLowerCase() === 'planning'
    }
  } catch (error) {
    console.error('Failed to update workflow agent:', error)
  }
}

  // 错误边界处理
  const onErrorCaptured = (err, instance, info) => {
    console.warn('[Workflow] UI error captured:', err.message, info)
    // 返回 false 阻止错误继续传播
    return false
  }

const submitAskUserResponse = async (content) => {
  if (!content?.trim()) return

  askUserSubmitting.value = true
  try {
    await coreOnSendMessage(content)
  } finally {
    askUserSubmitting.value = false
  }
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
}

const toggleWorkflowApprovalMute = async () => {
  await settingStore.setSetting('workflowApprovalMuted', !workflowApprovalMuted.value)
}

const handleApprovalCommand = async (sessionId) => {
  if (!sessionId) return
  await selectWorkflow(sessionId)
}

const resolveInitialWorkflowId = () => {
  const savedWorkflowId = settingStore.settings.workflowLastSelectedId
  if (
    savedWorkflowId &&
    workflowStore.workflows.some((workflow) => workflow.id === savedWorkflowId)
  ) {
    return savedWorkflowId
  }

  return workflowStore.workflows[0]?.id || null
}

const onGlobalKeyDown = (event) => {
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

// Watch for workflow changes to scroll
watch(
  () => workflowStore.messages,
  () => {
    nextTick(() => {
      messageListRef.value?.scrollToBottom()
    })
  },
  { deep: true }
)

onMounted(async () => {
  unlistenFocusInput.value = await listen('cs://workflow-focus-input', (event) => {
    if (event.payload && event.payload.windowLabel === settingStore.windowLabel) {
      inputAreaRef.value?.focus()
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

  if (agentStore.primaryAgents.length > 0) {
    selectedAgent.value = agentStore.primaryAgents[0]
  }

  // Restore the last selected workflow if it still exists.
  const initialWorkflowId = resolveInitialWorkflowId()
  if (initialWorkflowId) {
    await selectWorkflow(initialWorkflowId)
  } else {
    // First launch bootstrap: create one empty workflow so sending messages never hits "no session".
    await coreCreateNewWorkflow()
  }

  windowStore.initWorkflowWindowAlwaysOnTop()
  window.addEventListener('keydown', onGlobalKeyDown)
  window.addEventListener('resize', updateMaxWidth)

  // Initial scroll
  nextTick(() => messageListRef.value?.scrollToBottom(true))
})

onBeforeUnmount(() => {
  if (unlistenWorkflowEvents.value) {
    unlistenWorkflowEvents.value()
  }
  unlistenFocusInput.value?.()
  window.removeEventListener('keydown', onGlobalKeyDown)
  window.removeEventListener('resize', updateMaxWidth)
  clearRetryTimer()
})
</script>

<style lang="scss">
@use '@/styles/workflow/index' as *;

.workflow-titlebar-primary-path {
  max-width: min(40vw, 360px);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--cs-font-size-sm);
  font-weight: 500;
  color: var(--cs-text-primary);
}
</style>
