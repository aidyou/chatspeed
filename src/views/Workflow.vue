<template>
  <div class="workflow-layout">
    <Titlebar>
      <template #left>
        <el-tooltip :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)" placement="right"
          :hide-after="0" :enterable="false">
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
        </el-tooltip>
      </template>
      <template #center></template>
      <template #right>
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
          :is-chatting="isChatting" :chat-state="chatState" :is-compressing="isCompressing"
          :compression-message="compressionMessage" :last-assistant-message="lastAssistantMessage"
          :is-message-expanded="isMessageExpanded" :is-reasoning-expanded="isReasoningExpanded"
          :remove-system-reminder="removeSystemReminder" :get-diff-markdown="getDiffMarkdown"
          :parse-choice-content="parseChoiceContent" :get-parsed-message="getParsedMessage"
          :get-reasoning-preview="getReasoningPreview" @toggle-expand="toggleMessageExpand"
          @toggle-reasoning="toggleReasoningExpand" @send-choice="sendUserChoice" />

        <!-- Status Panel (Floating) -->
        <StatusPanel />

        <!-- Input Area -->
        <WorkflowInputArea ref="inputAreaRef" v-model:input-message="inputMessage" :is-running="isRunning"
          :is-awaiting-approval="isAwaitingApproval" :current-workflow="currentWorkflow"
          :current-workflow-id="currentWorkflowId" :selected-agent="selectedAgent"
          :active-model-name="activeModelName" :planning-mode="planningMode" :approval-level="approvalLevel"
          :final-audit-mode="finalAuditMode" :agents="agentStore.agents" :active-ask-user="activeAskUser"
          :show-skill-suggestions="showSkillSuggestions" :show-file-suggestions="showFileSuggestions"
          :filtered-system-skills="filteredSystemSkills" :file-suggestions="fileSuggestions"
          :selected-skill-index="selectedSkillIndex" :selected-file-index="selectedFileIndex"
          :on-input-key-down="onInputKeyDown" :on-composition-start="onCompositionStart"
          :on-composition-end="onCompositionEnd" :on-skill-select="onSkillSelect" :on-file-select="onFileSelect"
          @send-message="onSendMessage" @continue="onContinue" @stop="onStop" @approve-plan="onApprovePlan"
          @toggle-planning-mode="planningMode = !planningMode" @toggle-final-audit-mode="toggleFinalAuditMode"
          @update-approval-level="approvalLevel = $event" @create-new-workflow="createNewWorkflow"
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

    <ApprovalDialog v-model="approvalVisible" :action="approvalAction" :details="approvalDetails"
      :loading="approvalLoading" @approve="onApproveAction" @approve-all="onApproveAllAction" @reject="onRejectAction" />

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
import ApprovalDialog from '@/components/workflow/ApprovalDialog.vue'
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
const finalAuditMode = ref('on')
const planningMode = ref(false)

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
  getParsedMessage
} = useWorkflowMessages()

// Approval composable
const {
  approvalVisible,
  approvalAction,
  approvalDetails,
  approvalRequestId,
  approvalLoading,
  onApproveAction,
  onApproveAllAction,
  onRejectAction
} = useWorkflowApproval({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId)
})

// ============================================================
// Composables that DEPEND on local state
// ============================================================

// Paths composable - needs selectedAgent
const {
  pendingPaths,
  currentPaths,
  canEditPaths,
  onAddPathFromTree,
  onRemovePathFromTree
} = useWorkflowPaths({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId),
  selectedAgent: computed(() => selectedAgent.value)
})

// Input composable - needs currentPaths, systemSkills
const inputComposable = useWorkflowInput({
  inputRef: computed(() => inputAreaRef.value?.inputRef),
  onSendMessage: null, // Not used anymore
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
  approvalVisible,
  approvalRequestId,
  approvalAction,
  approvalDetails,
  enhancedMessages,
  isCompressing,
  compressionMessage,
  fetchSystemSkills,
  resetChatState,
  setRetryStatus,
  processChunk,
  processReasoningChunk,
  setCompressionStatus,
  scrollToBottom: () => messageListRef.value?.scrollToBottom()
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
  isAwaitingApproval,
  activeModelName,
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

// ============================================================
// Wrapper functions combining multiple composables
// ============================================================

// Wrapper function that handles input clearing after send
const onSendMessage = async () => {
  const message = inputMessage.value
  if (!message.trim()) return

  clearInput()
  const wasCommand = await coreOnSendMessage(message)
  return wasCommand
}

// Wrapper for createNewWorkflow that also clears input
const createNewWorkflow = () => {
  coreCreateNewWorkflow()
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

// ============================================================
// Computed properties
// ============================================================

const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)

const filteredWorkflows = computed(() => {
  const searchQuery = '' // From WorkflowSidebar component
  if (!searchQuery) return workflows.value
  return workflows.value.filter((wf) =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.toLowerCase())
  )
})

const activeAskUser = computed(() => {
  if (currentWorkflow.value?.status !== 'paused') return null
  const lastMsg = enhancedMessages.value[enhancedMessages.value.length - 1]
  if (
    lastMsg?.role === 'tool' &&
    lastMsg.toolDisplay?.action === 'Ask User' &&
    lastMsg.toolDisplay?.displayType === 'choice'
  ) {
    return parseChoiceContent(removeSystemReminder(lastMsg.message))
  }
  return null
})

const sendUserChoice = async (option) => {
  if (isRunning.value) return
  inputMessage.value = option
  onSendMessage()
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
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

// Watch for approval level changes
watch(approvalLevel, async (newVal) => {
  if (currentWorkflowId.value) {
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal: JSON.stringify({
        type: 'update_approval_level',
        level: newVal
      })
    })
    await workflowStore.selectWorkflow(currentWorkflowId.value)
  }
})

// Watch for final audit mode changes
watch(finalAuditMode, async (newVal) => {
  if (currentWorkflowId.value) {
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal: JSON.stringify({
        type: 'update_final_audit',
        audit: newVal === 'on'
      })
    })
    await workflowStore.selectWorkflow(currentWorkflowId.value)
  }
})

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

  if (agentStore.agents.length > 0) {
    selectedAgent.value = agentStore.agents[0]
  }

  // Load the last workflow if available
  if (workflowStore.workflows.length > 0) {
    await selectWorkflow(workflowStore.workflows[0].id)

    // Check if we need to show approval dialog after loading
    nextTick(() => {
      const lastMsg = enhancedMessages.value[enhancedMessages.value.length - 1]
      if (
        currentWorkflow.value?.status === 'awaiting_approval' &&
        lastMsg?.role === 'tool'
      ) {
        approvalRequestId.value = lastMsg.metadata?.tool_call_id || 'restored'
        approvalAction.value = lastMsg.toolDisplay.action
        approvalDetails.value = removeSystemReminder(lastMsg.message)
        approvalVisible.value = true
      }
    })
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
</style>
