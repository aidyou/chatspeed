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
          :queued-messages="workflowStore.messageQueue"
          :is-chatting="isChatting" :chat-state="chatState" :is-compressing="isCompressing"
          :compression-message="compressionMessage" :last-assistant-message="lastAssistantMessage"
          :is-message-expanded="isMessageExpanded" :is-reasoning-expanded="isReasoningExpanded"
          :remove-system-reminder="removeSystemReminder" :get-diff-markdown="getDiffMarkdown"
          :parse-choice-content="parseChoiceContent" :get-parsed-message="getParsedMessage"
          :get-reasoning-preview="getReasoningPreview" :should-show-tool-raw-content="shouldShowToolRawContent"
          @toggle-expand="toggleMessageExpand"
          @toggle-reasoning="toggleReasoningExpand" @send-choice="sendUserChoice" />

      <!-- Status Panel (Floating) -->
      <StatusPanel />

        <!-- Input Area -->
        <WorkflowInputArea ref="inputAreaRef" v-model:input-message="inputMessage" :is-running="isRunning"
          :has-live-session="hasLiveSession" :wait-reason="waitReason"
          :current-workflow="currentWorkflow"
          :current-workflow-id="currentWorkflowId" :selected-agent="selectedAgent" :can-edit-agent="canEditCurrentWorkflowAgent"
          :active-model-name="activeModelName"
          :planning-mode="planningMode" :approval-level="approvalLevel" :final-audit-mode="finalAuditMode"
          :agents="agentStore.agents" :active-ask-user="activeAskUser" :show-skill-suggestions="showSkillSuggestions"
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

    <ApprovalDialog v-model="approvalVisible" :action="approvalAction" :details="approvalDetails" :display-type="approvalDisplayType"
      :loading="approvalLoading" @approve="onApproveAction" @approve-all="onApproveAllAction"
      @reject="onRejectAction" @stop="onStop" />

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
  getParsedMessage,
  shouldShowToolRawContent
} = useWorkflowMessages()

// Approval composable
const {
  approvalVisible,
  approvalAction,
  approvalDetails,
  approvalDisplayType,
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
  approvalVisible,
  approvalRequestId,
  approvalAction,
  approvalDetails,
  approvalDisplayType,
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
    await invokeWrapper('update_workflow_agent_id', {
      sessionId: currentWorkflowId.value,
      agentId: agent.id
    })

    if (workflowStore.currentWorkflow) {
      workflowStore.currentWorkflow.agentId = agent.id
      workflowStore.currentWorkflow.agentConfig = {
        ...(workflowStore.currentWorkflow.agentConfig || {}),
        models: agent.models || null,
        allowedPaths: agent.allowedPaths || [],
        shellPolicy: agent.shellPolicy || [],
        approvalLevel: agent.approvalLevel || 'default',
        finalAudit: false
      }
    }
  } catch (error) {
    console.error('Failed to update workflow agent:', error)
  }
}

const isAskUserPromptMessage = (msg) => {
  if (!msg || msg.role !== 'tool') return false
  const meta = msg.metadata || {}
  const toolName = (
    meta.tool_name ||
    meta.tool_call?.name ||
    meta.tool_call?.function?.name ||
    ''
  ).toLowerCase()
  const title = (meta.title || '').toLowerCase()
  const summary = (meta.summary || '').toLowerCase()
  return toolName === 'ask_user' || title === 'ask user' || summary.includes('waiting for user')
}

const hasUserResponseAfter = (messages, fromIndex) => {
  for (let i = fromIndex + 1; i < messages.length; i++) {
    const msg = messages[i]
    if (msg?.role !== 'user') continue
    const content = (msg.message || '').trim()
    if (!content) continue
    if (content.includes('<SYSTEM_REMINDER>')) continue
    return true
  }
  return false
}

const activeAskUser = computed(() => {
  const rawMessages = workflowStore.messages || []
  for (let i = rawMessages.length - 1; i >= 0; i--) {
    const msg = rawMessages[i]
    if (!isAskUserPromptMessage(msg)) continue
    if (hasUserResponseAfter(rawMessages, i)) return null
    return parseChoiceContent(removeSystemReminder(msg.message || ''))
  }
  return null
})

  // 错误边界处理
  const onErrorCaptured = (err, instance, info) => {
    console.warn('[Workflow] UI error captured:', err.message, info)
    // 返回 false 阻止错误继续传播
    return false
  }

  const sendUserChoice = (option) => {
    inputMessage.value = option
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

// Watch for current workflow changes to sync approvalLevel
watch(
  () => workflowStore.currentWorkflow?.agentConfig,
  (newConfig) => {
    if (newConfig) {
      // Sync approvalLevel from workflow config
      if (newConfig.approvalLevel) {
        approvalLevel.value = newConfig.approvalLevel
      }
      // Sync finalAuditMode from workflow config
      if (newConfig.finalAudit !== undefined && newConfig.finalAudit !== null) {
        finalAuditMode.value = newConfig.finalAudit ? 'on' : 'off'
      }
    }
  },
  { immediate: true }
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

  // Load the last workflow if available
  if (workflowStore.workflows.length > 0) {
    await selectWorkflow(workflowStore.workflows[0].id)
    // Approval dialog will be shown via request_confirm_broadcast in selectWorkflow
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
</style>
