<template>
  <div class="workflow-layout">
    <Titlebar :show-menu-button="settingStore.settings.showMenuButton">
      <template #left>
        <div class="workflow-titlebar-left-actions">
          <el-tooltip
            :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)"
            placement="right"
            :hide-after="0"
            :enterable="false">
            <div class="icon-btn upperLayer" @click="onToggleSidebar">
              <cs name="sidebar" />
            </div>
          </el-tooltip>
          <el-tooltip
            :content="$t('workflow.automation.title')"
            :hide-after="0"
            :enterable="false"
            placement="bottom">
            <div class="icon-btn upperLayer" @click="openCreateAutomation">
              <cs name="clock" />
            </div>
          </el-tooltip>
        </div>
      </template>
      <template #center>
        <div
          v-if="displayAllowedPathTitle"
          class="workflow-titlebar-primary-path"
          :title="displayAllowedPathTitle">
          {{ displayAllowedPathTitle }}
        </div>
      </template>
      <template #right>
        <el-dropdown
          v-if="globalPendingApprovalList.length > 0"
          trigger="click"
          @command="handleApprovalCommand">
          <div class="icon-btn upperLayer approval-queue-btn blinking">
            <cs name="approval" />
            <span class="approval-queue-count">{{ approvalQueueCount }}</span>
          </div>
          <template #dropdown>
            <el-dropdown-menu class="approval-queue-menu">
              <el-dropdown-item
                v-for="item in globalPendingApprovalList"
                :key="item.key"
                :command="item.sessionId">
                <div class="approval-menu-item">
                  <div class="approval-menu-title">
                    <cs name="approval" size="var(--cs-font-size-md)" />
                    {{ getPendingApprovalTitle(item) }}
                  </div>
                  <div class="approval-menu-summary" :title="item.workflowTitle || item.action">
                    {{ item.workflowTitle || item.action }}
                  </div>
                </div>
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <el-tooltip
          v-if="updateStore.isUpdateReady"
          :content="$t('common.newVersionReady')"
          :hide-after="0"
          :enterable="false"
          placement="bottom">
          <div class="menu icon-btn upperLayer restart update-ready-btn" @click="updateStore.restartApp">
            <cs name="restart" />
            {{ $t('common.updateButtonText') }}
          </div>
        </el-tooltip>
        <el-dropdown trigger="click">
          <div class="icon-btn upperLayer">
            <el-tooltip
              :content="$t('workflow.notificationSound')"
              :hide-after="0"
              :enterable="false"
              placement="bottom">
              <cs :name="soundIcon" />
            </el-tooltip>
          </div>
          <template #dropdown>
            <el-dropdown-menu class="sound-dropdown-menu">
              <el-dropdown-item>
                <el-checkbox
                  :model-value="!workflowApprovalMuted"
                  @change="toggleWorkflowApprovalMute">
                  {{ $t('workflow.approvalSound') }}
                </el-checkbox>
              </el-dropdown-item>
              <el-dropdown-item>
                <el-checkbox
                  :model-value="!workflowCompletionMuted"
                  @change="toggleWorkflowCompletionMute">
                  {{ $t('workflow.completionSound') }}
                </el-checkbox>
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
        <div
          class="icon-btn upperLayer"
          :class="{ disabled: !canDeleteLastMessage }"
          @click="onDeleteLastMessage">
          <el-tooltip
            :content="$t('workflow.deleteLastMessage')"
            :hide-after="0"
            :enterable="false"
            placement="bottom">
            <cs name="undo" />
          </el-tooltip>
        </div>
        <div class="icon-btn upperLayer pin-btn" @click="onPin" :class="{ active: isAlwaysOnTop }">
          <el-tooltip
            :content="$t(`common.${isAlwaysOnTop ? 'unpin' : 'pin'}`)"
            :hide-after="0"
            :enterable="false"
            placement="bottom">
            <cs name="pin" />
          </el-tooltip>
        </div>
      </template>
    </Titlebar>

    <div class="workflow-main">
      <WorkflowSidebar
        :workflows="filteredWorkflows"
        :current-workflow-id="currentWorkflowId"
        :reset-primary-root-filter-token="sidebarRootFilterResetToken"
        :sidebar-collapsed="sidebarCollapsed"
        :sidebar-width="sidebarWidth"
        :sidebar-style="sidebarStyle"
        :current-paths="currentPaths"
        :can-switch-workflow="canSwitchWorkflow"
        :is-dragging="isDragging"
        :automations="workflowAutomationStore.automations"
        :selected-automation-id="workflowAutomationStore.selectedAutomationId"
        v-model:active-tab="workflowSidebarActiveTab"
        @select-workflow="onSelectWorkflowFromHistory"
        @select-automation="onSelectAutomation"
        @create-automation="openCreateAutomation"
        @edit-automation="onEditAutomation"
        @delete-automation="onDeleteAutomation"
        @edit-workflow="onEditWorkflow"
        @delete-workflow="onDeleteWorkflow"
        @add-path-from-tree="onAddPathFromTree"
        @remove-path-from-tree="onRemovePathFromTree" />

      <!-- Resize Handle -->
      <div
        v-if="!sidebarCollapsed"
        class="sidebar-resize-handle"
        :class="{ dragging: isDragging }"
        @mousedown="onResizeStart" />

      <!-- Main container -->
      <el-container class="main-container">
        <el-main class="message-list-container">
          <WorkflowMessageList
            :key="currentWorkflowId || 'workflow-empty'"
            ref="messageListRef"
            :messages="enhancedMessages"
            :hidden-completed-task-group-count="hiddenCompletedTaskGroupCount"
            :is-running="isRunning"
            :queued-messages="workflowStore.messageQueue"
            :is-chatting="isChatting"
            :chat-state="chatState"
            :is-compressing="isCompressing"
            :compression-message="compressionMessage"
            :last-assistant-message="lastAssistantMessage"
            :approval-loading="approvalLoading"
            :active-approval-id="activeApprovalId"
            :ask-user-submitting="askUserSubmitting"
            :is-message-expanded="isMessageExpanded"
            :is-reasoning-expanded="isReasoningExpanded"
            :remove-system-reminder="removeSystemReminder"
            :get-diff-markdown="getDiffMarkdown"
            :parse-choice-content="parseChoiceContent"
            :get-parsed-message="getParsedMessage"
            :should-show-tool-raw-content="shouldShowToolRawContent"
            :pending-count="currentInlinePendingApprovalIds.length"
            :pending-approval-ids="currentInlinePendingApprovalIds"
            :current-workflow-id="currentWorkflowId"
            :is-approval-submitting="isApprovalSubmitting"
            @toggle-expand="toggleMessageExpand"
            @toggle-reasoning="toggleReasoningExpand"
            @reveal-earlier-task-group="visibleCompletedTaskGroupCount += 1"
            @submit-ask-user="submitAskUserResponse"
            @approve-tool="onApproveAction"
            @approve-all-tool="onApproveAllAction"
            @approve-all-pending="onApproveAllPendingAction"
            @remove-queued-message="removeQueuedMessage"
            @reject-tool="onRejectAction" />
        </el-main>

        <!-- Status Panel (Floating) -->
        <StatusPanel />

        <!-- Input Area -->
        <WorkflowInputArea
          ref="inputAreaRef"
          v-model:input-message="inputMessage"
          :is-running="isRunning"
          :is-chatting="isChatting"
          :has-live-session="hasLiveSession"
          :chat-state="chatState"
          :wait-reason="waitReason"
          :current-workflow="currentWorkflow"
          :current-workflow-id="currentWorkflowId"
          :selected-agent="selectedAgent"
          :can-edit-agent="canEditCurrentWorkflowAgent"
          :show-planning-mode-toggle="showPlanningModeToggle"
          :can-toggle-planning-mode="canEditCurrentWorkflowAgent"
          :active-model-name="activeModelName"
          :planning-mode="planningMode"
          :approval-level="approvalLevel"
          :final-audit-mode="finalAuditMode"
          :can-toggle-final-audit-mode="canToggleFinalAuditMode"
          :auto-compress-enabled="autoCompressEnabled"
          :agents="agentStore.agents"
          :attachments="imageAttachments"
          :can-attach-images="canUseImageAttachments"
          :is-preparing-image-send="isPreparingImageSend"
          :show-skill-suggestions="showSkillSuggestions"
          :show-file-suggestions="showFileSuggestions"
          :filtered-system-skills="filteredSystemSkills"
          :grouped-skill-suggestions="groupedSkillSuggestions"
          :file-suggestions="fileSuggestions"
          :selected-skill-index="selectedSkillIndex"
          :selected-file-index="selectedFileIndex"
          :on-input-key-down="onInputKeyDown"
          :on-composition-start="onCompositionStart"
          :on-composition-end="onCompositionEnd"
          :on-paste-input="onImagePaste"
          :on-skill-select="onSkillSelect"
          :on-file-select="onFileSelect"
          @send-message="onSendMessage"
          @continue="onContinue"
          @stop="onStop"
          @approve-plan="onApprovePlan"
          @toggle-planning-mode="onTogglePlanningMode"
          @toggle-final-audit-mode="toggleFinalAuditMode"
          @toggle-auto-compress="autoCompressEnabled = !autoCompressEnabled"
          @update-approval-level="approvalLevel = $event"
          @update-selected-agent="onSelectedAgentChange"
          @create-new-workflow="createNewWorkflow"
          @open-image-dialog="openImageAttachmentDialog"
          @open-model-selector="openModelSelector"
          @remove-attachment="removeImageAttachment"
          @open-skills-selector="openSkillsSelector" />
      </el-container>
    </div>

    <!-- Edit workflow dialog -->
    <el-dialog
      v-model="editWorkflowDialogVisible"
      :title="$t('workflow.editWorkflowTitle')"
      :close-on-press-escape="false"
      width="50%">
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

    <WorkflowModelSelector
      v-model="modelSelectorVisible"
      :initial-tab="modelSelectorTab"
      :agent="selectedAgent"
      @save="onModelConfigSave" />

    <WorkflowSkillsSelector
      v-model="skillsSelectorVisible"
      :current-workflow="currentWorkflow"
      :agent="selectedAgent"
      :system-skills="systemSkills"
      @save="onSkillsConfigSave" />

    <WorkflowAutomationEditor
      v-model="automationDrawerVisible"
      @saved="onAutomationSaved"
      @started-workflow="onAutomationStartedWorkflow" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import { ElMessageBox } from 'element-plus'
import { invokeWrapper } from '@/libs/tauri'
import { imagePreview, imageSourceUrl } from '@/libs/fs'
import { showMessage, Uuid } from '@/libs/util'

import { useWorkflowStore } from '@/stores/workflow'
import { useWorkflowAutomationStore } from '@/stores/workflowAutomation'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useUpdateStore } from '@/stores/update'
import { useWindowStore } from '@/stores/window'

import Titlebar from '@/components/window/Titlebar.vue'
import StatusPanel from '@/components/workflow/StatusPanel.vue'
import WorkflowModelSelector from '@/components/workflow/WorkflowModelSelector.vue'
import WorkflowSkillsSelector from '@/components/workflow/WorkflowSkillsSelector.vue'
import WorkflowSidebar from '@/components/workflow/WorkflowSidebar.vue'
import WorkflowMessageList from '@/components/workflow/WorkflowMessageList.vue'
import WorkflowInputArea from '@/components/workflow/WorkflowInputArea.vue'
import WorkflowAutomationEditor from '@/components/workflow/automation/WorkflowAutomationEditor.vue'

// Composables
import { useWorkflowSidebar } from '@/composables/workflow/useWorkflowSidebar'
import { useWorkflowChat } from '@/composables/workflow/useWorkflowChat'
import { useWorkflowMessages } from '@/composables/workflow/useWorkflowMessages'
import { useWorkflowApproval } from '@/composables/workflow/useWorkflowApproval'
import { useWorkflowPaths } from '@/composables/workflow/useWorkflowPaths'
import { useWorkflowInput } from '@/composables/workflow/useWorkflowInput'
import { useWorkflowCore } from '@/composables/workflow/useWorkflowCore'
import { TERMINAL_STATUSES } from '@/composables/workflow/signalTypes'

const IMAGE_FILE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'svg'])

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const workflowAutomationStore = useWorkflowAutomationStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const updateStore = useUpdateStore()
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
const autoCompressEnabled = ref(true)
const imageAttachments = ref([])
const defaultImageRecognitionPrompt = ref('')
const visibleCompletedTaskGroupCount = ref(3)
const automationDrawerVisible = ref(false)
const workflowSidebarActiveTab = ref('history')
const lastHistoryWorkflowId = ref(null)

const showPlanningModeToggle = computed(() => {
  const workflow = workflowStore.currentWorkflow
  if (!workflow) return true

  const hasStartedContent =
    Boolean(String(workflow.userQuery || '').trim()) || (workflow.messagesCount || 0) > 0
  const status = String(workflow.status || '').toLowerCase()
  return !workflowStore.hasLiveSession && (!hasStartedContent || TERMINAL_STATUSES.includes(status))
})

// System skills
const systemSkills = ref([])
const skillsSelectorVisible = ref(false)
const ALWAYS_ENABLED_SKILL_NAMES = ['help']
const fetchSystemSkills = async () => {
  try {
    const result = await invokeWrapper('get_system_skills')
    systemSkills.value = result || []
  } catch (error) {
    console.error('Failed to fetch system skills:', error)
  }
}

const activeSkillAgent = computed(() => {
  const workflowAgentId = workflowStore.currentWorkflow?.agentId
  if (workflowAgentId) {
    return agentStore.agents.find(agent => agent.id === workflowAgentId) || selectedAgent.value
  }
  return selectedAgent.value
})

const workflowSkillConfigSource = computed(() => {
  if (workflowStore.currentWorkflow?.agentConfig) {
    return workflowStore.currentWorkflow.agentConfig
  }
  return activeSkillAgent.value
})

const workflowInputSkills = computed(() => {
  const source = workflowSkillConfigSource.value
  if (!source || source.skillEnabled === false) return []

  const configuredSelectedSkills = Array.isArray(source.selectedSkills)
    ? source.selectedSkills
    : null
  if (configuredSelectedSkills === null) {
    return systemSkills.value
  }

  const allowedNames = new Set([...configuredSelectedSkills, ...ALWAYS_ENABLED_SKILL_NAMES])
  return systemSkills.value.filter(skill => allowedNames.has(skill.name))
})

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
  resetChatState,
  setRetryStatus,
  processChunk,
  processReasoningChunk,
  setCompressionStatus
} = useWorkflowChat({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId)
})

// Messages composable
const {
  expandedMessages,
  expandedReasonings,
  enhancedMessages,
  hiddenCompletedTaskGroupCount,
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
} = useWorkflowMessages({
  visibleCompletedTaskGroupCount
})

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
  selectedAgent: computed(() => selectedAgent.value),
  activeTab: computed(() => workflowSidebarActiveTab.value),
  selectedAutomation: computed(() => workflowAutomationStore.selectedAutomation),
  historyItemCount: computed(() => filteredWorkflows.value.length),
  automationItemCount: computed(() => workflowAutomationStore.automations.length)
})

// Input composable - needs currentPaths, systemSkills
const inputComposable = useWorkflowInput({
  inputRef: computed(() => inputAreaRef.value?.inputRef),
  onSendMessage: null, // Will be set after core composable is initialized
  currentPaths: computed(() => currentPaths.value),
  systemSkills: computed(() => workflowInputSkills.value),
  onImageFileSelect: async file =>
    (await addImageAttachmentFromPath(file.path, file.relative_path)) ? 'handled' : 'blocked'
})

const {
  inputMessage,
  showSkillSuggestions,
  showFileSuggestions,
  selectedSkillIndex,
  selectedFileIndex,
  fileSuggestions,
  filteredSystemSkills,
  groupedSkillSuggestions,
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
  autoCompressEnabled,
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
  openSkillsSelector: async () => {
    await fetchSystemSkills()
    skillsSelectorVisible.value = true
  },
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
  hasBlockingLiveSession,
  canRewindTail,
  waitReason,
  canStop,
  canContinue,
  activeModelName,
  canToggleFinalAuditMode,
  pendingApprovalList,
  getPendingApprovalEntry,
  clearPendingApprovalEntry,
  upsertPendingApprovalEntry,
  canSwitchWorkflow,
  selectWorkflow,
  startNewWorkflow,
  onSendMessage: coreOnSendMessage,
  removeQueuedMessage,
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
  isApprovalSubmitting,
  onApproveAction,
  onApproveAllAction,
  onRejectAction
} = useWorkflowApproval({
  currentWorkflowId: computed(() => workflowStore.currentWorkflowId),
  getPendingApprovalEntry,
  clearPendingApprovalEntry,
  upsertPendingApprovalEntry
})

function normalizeVisionModel(model) {
  if (!model || !model.id || !model.model) {
    return null
  }

  return {
    id: model.id,
    model: model.model
  }
}

const activeVisionModel = computed(() => {
  const workflowModel = normalizeVisionModel(currentWorkflow.value?.agentConfig?.models?.vision)
  if (workflowModel) {
    return workflowModel
  }

  const agentModel = normalizeVisionModel(selectedAgent.value?.visionModel)
  if (agentModel) {
    return agentModel
  }

  return normalizeVisionModel(settingStore.settings.visionModel)
})

const activeImageRecognitionPrompt = computed(() => {
  const workflowPrompt = String(
    currentWorkflow.value?.agentConfig?.imageRecognitionPrompt || ''
  ).trim()
  if (workflowPrompt) {
    return workflowPrompt
  }

  const agentPrompt = String(selectedAgent.value?.imageRecognitionPrompt || '').trim()
  if (agentPrompt) {
    return agentPrompt
  }

  return defaultImageRecognitionPrompt.value
})

const canUseImageAttachments = computed(() => !!activeVisionModel.value)
const isPreparingImageSend = ref(false)

function generateAttachmentId() {
  return `workflow_attachment_${Uuid()}`
}

function createPendingImageAttachment(attachment) {
  const pendingAttachment = {
    id: generateAttachmentId(),
    type: 'image',
    uploading: true,
    ...attachment
  }
  imageAttachments.value.push(pendingAttachment)
  return pendingAttachment
}

function updateImageAttachment(id, updates) {
  const attachment = imageAttachments.value.find(item => item.id === id)
  if (!attachment) {
    return false
  }

  Object.assign(attachment, updates)
  return true
}

function removeImageAttachment(id) {
  const index = imageAttachments.value.findIndex(attachment => attachment.id === id)
  if (index > -1) {
    imageAttachments.value.splice(index, 1)
  }
}

function clearImageAttachments() {
  imageAttachments.value = []
}

async function addImageAttachmentFromPath(path, name = '') {
  if (!canUseImageAttachments.value) {
    showMessage(t('settings.general.visionModelRequired'), 'warning')
    return false
  }

  const pendingAttachment = createPendingImageAttachment({
    name: String(name || path.split(/[/\\]/).pop() || 'image'),
    path,
    size: 0
  })

  try {
    const [previewUrl, sourceUrl] = await Promise.all([imagePreview(path), imageSourceUrl(path)])
    if (!previewUrl || !sourceUrl) {
      throw new Error(t('chat.unsupportedFileType'))
    }

    updateImageAttachment(pendingAttachment.id, {
      url: previewUrl,
      sourceUrl,
      uploading: false
    })
    return true
  } catch (error) {
    removeImageAttachment(pendingAttachment.id)
    console.error('Failed to add workflow image attachment from path:', error)
    showMessage(t('chat.errorOnAddAttachment', { error: error.message || String(error) }), 'error')
    return false
  }
}

async function addImageAttachmentFromFile(file) {
  if (!canUseImageAttachments.value) {
    showMessage(t('settings.general.visionModelRequired'), 'warning')
    return false
  }

  let pendingAttachment = null

  try {
    const rawFile = file.raw || file
    pendingAttachment = createPendingImageAttachment({
      name: rawFile.name,
      size: rawFile.size
    })
    const url = await new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = event => resolve(event.target?.result)
      reader.onerror = reject
      reader.readAsDataURL(rawFile)
    })

    if (!url) {
      throw new Error(t('chat.unsupportedFileType'))
    }

    updateImageAttachment(pendingAttachment.id, {
      url,
      sourceUrl: url,
      uploading: false
    })
    return true
  } catch (error) {
    if (typeof pendingAttachment?.id === 'string') {
      removeImageAttachment(pendingAttachment.id)
    }
    console.error('Failed to add workflow image attachment:', error)
    showMessage(t('chat.errorOnAddAttachment', { error: error.message || String(error) }), 'error')
    return false
  }
}

async function onImagePaste(event) {
  if (!canUseImageAttachments.value) {
    return
  }

  const items = event.clipboardData?.items
  if (!items) {
    return
  }

  const imageFiles = []
  for (const item of items) {
    if (item.type.startsWith('image/')) {
      const file = item.getAsFile()
      if (file) {
        imageFiles.push(file)
      }
    }
  }

  if (!imageFiles.length) {
    return
  }

  event.preventDefault()
  for (const file of imageFiles) {
    await addImageAttachmentFromFile(file)
  }
}

async function openImageAttachmentDialog() {
  if (!canUseImageAttachments.value) {
    return
  }

  const selected = await open({
    multiple: true,
    filters: [
      {
        name: 'Images',
        extensions: Array.from(IMAGE_FILE_EXTENSIONS)
      }
    ]
  })

  const paths = Array.isArray(selected) ? selected : selected ? [selected] : []
  for (const path of paths) {
    await addImageAttachmentFromPath(path)
  }
}

async function analyzeImageAttachments(attachments, userMessage) {
  const visionModel = activeVisionModel.value
  if (!visionModel?.id || !visionModel?.model) {
    throw new Error(t('settings.general.visionModelRequired'))
  }

  const promptParts = [activeImageRecognitionPrompt.value]
  if (userMessage) {
    promptParts.push(`Current user request:\n${userMessage}`)
  }

  const visionMessage = {
    role: 'user',
    content: [{ type: 'text', text: promptParts.join('\n\n') }]
  }

  for (const attachment of attachments) {
    visionMessage.content.push({
      type: 'image_url',
      image_url: { url: attachment.sourceUrl || attachment.url }
    })
  }

  const visionChatId = `workflow_vision_${Uuid()}`
  chatState.value.step = t('chat.analyzingImages')
  isChatting.value = true

  let timeoutId = null
  let unlistenFn = null

  const normalizeVisionErrorMessage = error => {
    const raw = String(error?.message || error || '').trim()
    if (!raw) {
      return 'Vision analysis failed'
    }

    const extractStructuredErrorMessage = value => {
      const trimmed = String(value || '').trim()
      if (!trimmed || (!trimmed.startsWith('{') && !trimmed.startsWith('['))) {
        return ''
      }

      try {
        const parsed = JSON.parse(trimmed)
        if (parsed && typeof parsed === 'object') {
          const parsedMessage = String(parsed.message || parsed.error || '').trim()
          const parsedStatus = String(parsed.status || parsed.code || '').trim()
          if (parsedMessage && parsedStatus) {
            return `${parsedMessage} (status: ${parsedStatus})`
          }
          if (parsedMessage) {
            return parsedMessage
          }
        }
      } catch {
        return ''
      }

      return ''
    }

    const structuredErrorMessage = extractStructuredErrorMessage(raw)
    if (structuredErrorMessage) {
      return structuredErrorMessage
    }

    const sizeMatch = raw.match(
      /input size exceed limit\s+(\d+)x(\d+),\s*current input:\((\d+),\s*(\d+)\)/i
    )
    if (sizeMatch) {
      const [, limitW, limitH, currentW, currentH] = sizeMatch
      return t('chat.errorOnAddAttachment', {
        error: `Image size ${currentW}x${currentH} exceeds model limit ${limitW}x${limitH}`
      })
    }

    return raw
  }

  try {
    const result = await new Promise(async (resolve, reject) => {
      let fullContent = ''
      let finished = false

      const rejectOnce = error => {
        if (finished) return
        finished = true
        reject(error)
      }

      try {
        unlistenFn = await listen('chat_stream', event => {
          const payload = event.payload
          const payloadChatId = payload.chatId || payload.chat_id
          if (payloadChatId !== visionChatId) {
            return
          }

          if (payload.type === 'text' && payload.chunk) {
            fullContent += payload.chunk
            return
          }

          if (payload.type === 'finished') {
            finished = true
            resolve(fullContent.trim())
            return
          }

          if (payload.type === 'error') {
            rejectOnce(new Error(normalizeVisionErrorMessage(payload.chunk || payload.message)))
          }
        })
      } catch (error) {
        reject(error)
        return
      }

      timeoutId = window.setTimeout(() => {
        if (!finished) {
          rejectOnce(new Error('Vision analysis timeout'))
        }
      }, 60000)

      try {
        await invokeWrapper('chat_completion', {
          providerId: visionModel.id,
          model: visionModel.model,
          chatId: visionChatId,
          messages: [visionMessage],
          networkEnabled: false,
          mcpEnabled: false,
          stream: false,
          toolsEnabled: false,
          metadata: {}
        })
      } catch (error) {
        rejectOnce(new Error(normalizeVisionErrorMessage(error)))
      }
    })

    return result
  } finally {
    if (timeoutId) {
      window.clearTimeout(timeoutId)
    }
    if (unlistenFn) {
      unlistenFn()
    }
    isChatting.value = false
  }
}

function buildImageAttachedContext(imageAnalysis, userMessage) {
  const escapeTagContent = value =>
    String(value || '')
      .replaceAll('&', '&amp;')
      .replaceAll('<', '&lt;')
      .replaceAll('>', '&gt;')
  const reminder =
    "Content inside the `<img_detail>` tag provides detailed information extracted from the user's image. Use it only as reference to assist in fulfilling the user's request, and do not treat it as the user's original input."
  const userQuery = escapeTagContent(userMessage)
  const imageDetail = escapeTagContent(imageAnalysis)
  return `<img_detail>${imageDetail}</img_detail><SYSTEM_REMINDER>${reminder}</SYSTEM_REMINDER><user_query>${userQuery}</user_query>`
}

function buildImageAttachmentMetadata(attachments) {
  return {
    attachments: attachments.map(attachment => ({
      type: 'image',
      name: attachment.name,
      size: attachment.size || 0,
      url: attachment.url,
      sourceUrl: attachment.sourceUrl || attachment.url
    }))
  }
}

function clearImageAnalysisErrorMessages() {
  workflowStore.removeCurrentWorkflowMessages(message => {
    const errorType = String(
      message?.metadata?.error_type || message?.metadata?.errorType || message?.errorType || ''
    ).trim()
    return errorType === 'image_analysis_error'
  })
}

function appendImageAnalysisErrorMessage(error, attachments = []) {
  const sessionId = workflowStore.currentWorkflowId
  if (!sessionId) {
    return
  }

  const errorMessage = String(
    error?.message || t('chat.errorOnAddAttachment', { error: String(error) })
  ).trim()

  workflowStore.addMessage({
    sessionId,
    role: 'assistant',
    message: errorMessage,
    stepType: 'Observe',
    stepIndex: workflowStore.messages.length,
    isError: true,
    errorType: 'image_analysis_error',
    metadata: {
      ...buildImageAttachmentMetadata(attachments),
      error_type: 'image_analysis_error',
      is_error: true
    }
  })
}

function buildPendingImageQueueText(message, attachments) {
  if (message) {
    return message
  }

  const names = attachments
    .map(attachment => String(attachment.name || '').trim())
    .filter(Boolean)
    .slice(0, 2)

  return names.join(', ') || t('chat.preparingAttachments')
}

function buildPendingQueueAttachments(attachments) {
  return attachments.map(attachment => ({
    id: attachment.id,
    type: attachment.type || 'image',
    name: attachment.name,
    url: attachment.url || attachment.sourceUrl || '',
    sourceUrl: attachment.sourceUrl || attachment.url || ''
  }))
}

function scrollMessageListToBottom(force = true) {
  nextTick(() => messageListRef.value?.scrollToBottom(force))
}

// Set up the onSendMessage callback for the input composable
inputComposable.onSendMessage.value = async () => {
  if (isPreparingImageSend.value) {
    return false
  }

  const backupMessage = inputMessage.value
  const backupAttachments = [...imageAttachments.value]
  const rawMessage = backupMessage.trim()

  if (!rawMessage && backupAttachments.length === 0) {
    return
  }

  let attachedContext = null
  let metadata = null
  let preparingQueueId = null

  try {
    clearImageAnalysisErrorMessages()

    if (backupAttachments.length > 0) {
      preparingQueueId = `local_queue_prepare_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`
      workflowStore.addMessageToQueue({
        id: preparingQueueId,
        content: buildPendingImageQueueText(rawMessage, backupAttachments),
        status: 'preparing_attachments',
        statusText: t('chat.analyzingImages'),
        attachments: buildPendingQueueAttachments(backupAttachments),
        removable: false,
        icon: 'loading'
      })
      scrollMessageListToBottom()
      clearInput()
      clearImageAttachments()
      isPreparingImageSend.value = true
      scrollMessageListToBottom()
      const imageAnalysis = await analyzeImageAttachments(backupAttachments, rawMessage)
      if (imageAnalysis) {
        attachedContext = buildImageAttachedContext(imageAnalysis, rawMessage)
        metadata = buildImageAttachmentMetadata(backupAttachments)
      }
    }
  } catch (error) {
    console.error('Failed to analyze workflow images:', error)
    if (preparingQueueId) {
      workflowStore.removeQueuedMessage(preparingQueueId)
    }
    appendImageAnalysisErrorMessage(error, backupAttachments)
    inputMessage.value = backupMessage
    imageAttachments.value = backupAttachments
    resetChatState()
    isChatting.value = false
    isPreparingImageSend.value = false
    showMessage(error?.message || t('chat.errorOnAddAttachment', { error: String(error) }), 'error')
    scrollMessageListToBottom()
    return
  }

  if (preparingQueueId) {
    workflowStore.removeQueuedMessage(preparingQueueId)
  } else {
    clearInput()
    clearImageAttachments()
  }
  isPreparingImageSend.value = false

  const wasCommand = await coreOnSendMessage(rawMessage, {
    attachedContext,
    metadata
  })
  if (wasCommand === false) {
    inputMessage.value = backupMessage
    imageAttachments.value = backupAttachments
  }
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
  clearImageAttachments()
}

// Wrapper for skill select that properly handles send
const onSkillSelect = skill => {
  originalOnSkillSelect(skill)
  // If it was a command (UI action), the input now contains the command
  // We need to trigger send manually since originalOnSkillSelect doesn't have access to onSendMessage
  if (skill.type === 'command') {
    onSendMessage()
  }
}

const openSkillsSelector = async () => {
  if (!currentWorkflowId.value && !selectedAgent.value) {
    showMessage(t('workflow.noAgentError'), 'warning')
    return
  }
  await fetchSystemSkills()
  skillsSelectorVisible.value = true
}

const onSkillsConfigSave = async config => {
  try {
    if (currentWorkflowId.value) {
      await invokeWrapper('update_workflow_skills_config', {
        sessionId: currentWorkflowId.value,
        skillEnabled: config.skillEnabled !== false,
        selectedSkills: config.selectedSkills || []
      })
      await workflowStore.selectWorkflow(currentWorkflowId.value)
    } else if (selectedAgent.value) {
      const updatedAgent = {
        ...selectedAgent.value,
        skillEnabled: config.skillEnabled !== false,
        selectedSkills: config.selectedSkills || []
      }
      await agentStore.saveAgent(updatedAgent)
      await agentStore.fetchAgents()
      selectedAgent.value =
        agentStore.agents.find(agent => agent.id === updatedAgent.id) || updatedAgent
    }

    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    console.error('Failed to save workflow skills config:', error)
    if (currentWorkflowId.value) {
      await workflowStore.selectWorkflow(currentWorkflowId.value)
    }
    showMessage(t('common.saveFailed'), 'error')
  }
}

// Approve all pending approval items for the current workflow using the
// in-message FIFO order so the inline item that triggered the batch action
// is never dropped from the snapshot.
const onApproveAllPendingAction = async payload => {
  const sessionId = currentWorkflowId.value
  if (!sessionId) return

  const startingToolCallId =
    typeof payload === 'string' ? payload : payload?.startingToolCallId || ''
  const preferredIds = Array.isArray(payload?.orderedToolCallIds) ? payload.orderedToolCallIds : []
  const orderedIds = []
  const seen = new Set()

  for (const toolCallId of preferredIds) {
    if (!toolCallId || seen.has(toolCallId)) continue
    seen.add(toolCallId)
    orderedIds.push(toolCallId)
  }

  for (const toolCallId of currentInlinePendingApprovalIds.value) {
    if (!toolCallId || seen.has(toolCallId)) continue

    seen.add(toolCallId)
    orderedIds.push(toolCallId)
  }

  if (startingToolCallId && !seen.has(startingToolCallId)) {
    orderedIds.unshift(startingToolCallId)
  }

  if (!orderedIds.length) return

  // Always resolve approvals sequentially against a stable snapshot.
  // The backend remains authoritative for pending approval order/state, and
  // concurrent approval signals can race with per-tool state transitions.
  for (const toolCallId of orderedIds) {
    await onApproveAction(toolCallId, sessionId)
  }
}

const openCreateAutomation = () => {
  workflowAutomationStore.selectAutomation(null)
  automationDrawerVisible.value = true
}

const onEditAutomation = async automationId => {
  workflowAutomationStore.selectAutomation(automationId)
  automationDrawerVisible.value = true
}

const resolveInitialAutomationId = () => {
  const savedAutomationId = settingStore.settings.workflowAutomationLastSelectedId
  if (
    savedAutomationId &&
    workflowAutomationStore.automations.some(automation => automation.id === savedAutomationId)
  ) {
    return savedAutomationId
  }

  return workflowAutomationStore.automations[0]?.id || null
}

const resolveAutomationWorkflowId = async automationId => {
  if (!automationId) return null

  const automation = workflowAutomationStore.automations.find(item => item.id === automationId)
  if (automation?.currentWorkflowSessionId) {
    return automation.currentWorkflowSessionId
  }

  const runs = await workflowAutomationStore.fetchRuns(automationId)
  const run = (runs || []).find(item => item?.workflowSessionId)
  return run?.workflowSessionId || null
}

const onSelectWorkflowFromHistory = async workflowId => {
  if (
    workflowSidebarActiveTab.value === 'history' &&
    currentWorkflowId.value === workflowId &&
    currentWorkflow.value?.isAutomationRun !== true
  ) {
    return
  }

  lastHistoryWorkflowId.value = workflowId || null
  await selectWorkflow(workflowId)
}

const onSelectAutomation = async automationId => {
  if (!automationId) return
  const workflowSessionId = await resolveAutomationWorkflowId(automationId)

  if (
    workflowSidebarActiveTab.value === 'automation' &&
    workflowAutomationStore.selectedAutomationId === automationId &&
    currentWorkflowId.value === workflowSessionId
  ) {
    return
  }

  workflowAutomationStore.selectAutomation(automationId)

  if (workflowSessionId) {
    try {
      await selectWorkflow(workflowSessionId)
      return
    } catch (error) {
      console.warn('[WorkflowAutomation] Failed to load linked workflow session:', error)
    }
  }

  workflowStore.clearCurrentWorkflow()
}

const onDeleteAutomation = async automationId => {
  const deletingSelectedAutomation =
    workflowAutomationStore.selectedAutomationId === automationId

  try {
    await ElMessageBox.confirm(
      t('workflow.automation.deleteConfirm'),
      t('workflow.automation.delete'),
      {
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      }
    )
  } catch {
    return
  }

  try {
    await workflowAutomationStore.deleteAutomation(automationId)
    workflowSidebarActiveTab.value = 'automation'
    await workflowAutomationStore.fetchAutomations()
    showMessage(t('common.deleteSuccess'), 'success')
    if (deletingSelectedAutomation) {
      automationDrawerVisible.value = false
      const nextAutomationId = workflowAutomationStore.selectedAutomationId || resolveInitialAutomationId()
      if (nextAutomationId) {
        await onSelectAutomation(nextAutomationId)
      } else {
        workflowStore.clearCurrentWorkflow()
      }
    }
  } catch (error) {
    showMessage(error?.message || String(error), 'error')
  }
}

const onAutomationSaved = async () => {
  workflowSidebarActiveTab.value = 'automation'
  await workflowAutomationStore.fetchAutomations()
  if (workflowAutomationStore.selectedAutomationId) {
    await onSelectAutomation(workflowAutomationStore.selectedAutomationId)
  }
}

const onAutomationStartedWorkflow = async workflowSessionId => {
  await workflowStore.loadWorkflows()
  await selectWorkflow(workflowSessionId)
}

// ============================================================
// Computed properties
// ============================================================

const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)
const workflowApprovalMuted = computed(() => !!settingStore.settings.workflowApprovalMuted)
const workflowCompletionMuted = computed(() => !!settingStore.settings.workflowCompletionMuted)
const soundIcon = computed(() => {
  // Show mute icon when both sounds are muted, otherwise show unmute/sound icon
  return workflowApprovalMuted.value && workflowCompletionMuted.value ? 'mute' : 'unmute'
})
const approvalQueueCount = computed(() => {
  const count = globalPendingApprovalList.value.length
  return count > 9 ? '9+' : String(count)
})
const sidebarRootFilterResetToken = ref(0)

const currentInlinePendingApprovalIds = computed(() => workflowStore.currentInlinePendingApprovalIds)
const globalPendingApprovalList = computed(() => {
  const activeSessionId = currentWorkflowId.value
  const backgroundEntries = pendingApprovalList.value.filter(
    entry => entry.sessionId !== activeSessionId && ['approval', 'ask_user'].includes(entry?.kind)
  )
  const currentEntries = workflowStore.currentInlinePendingApprovals
    .map(entry => ({
      ...entry,
      kind: 'approval'
    }))
    .filter(entry => entry?.kind === 'approval')
  const merged = [...currentEntries, ...backgroundEntries]
  const deduped = []
  const seen = new Set()

  for (const entry of merged) {
    const key = `${entry?.sessionId || ''}:${entry?.id || ''}`
    if (!entry || seen.has(key)) continue
    seen.add(key)
    deduped.push(entry)
  }

  return deduped
})
const canDeleteLastMessage = computed(() => {
  if (!currentWorkflowId.value) return false
  if (hasBlockingLiveSession.value) return false
  return canRewindTail.value === true
})

const displayAllowedPathTitle = computed(() => {
  if (!currentPaths.value?.length) return ''
  return displayAllowedPath.value || ''
})

const onDeleteLastMessage = async () => {
  if (!canDeleteLastMessage.value || !currentWorkflowId.value) return

  try {
    await ElMessageBox.confirm(
      t('workflow.deleteLastMessageConfirm'),
      t('workflow.deleteLastMessage'),
      {
        confirmButtonText: t('common.delete'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      }
    )
  } catch {
    return
  }

  try {
    const deleted = await invokeWrapper('delete_last_workflow_message', {
      sessionId: currentWorkflowId.value
    })

    if (!deleted) {
      showMessage(t('workflow.deleteLastMessageMissing'), 'warning')
      return
    }

    workflowStore.resetWorkflowUiProjection(currentWorkflowId.value)
    await selectWorkflow(currentWorkflowId.value)
    showMessage(t('workflow.deleteLastMessageDone'), 'success')
  } catch (error) {
    console.error('Failed to delete last workflow message:', error)
    showMessage(t('workflow.deleteLastMessageFailed', { error: String(error) }), 'error')
  }
}

const getWorkflowSortTime = workflow => {
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
    : workflows.value.filter(wf =>
        (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.toLowerCase())
      )

  return [...base].sort((a, b) => getWorkflowSortTime(b) - getWorkflowSortTime(a))
    .filter(wf => wf?.isAutomationRun !== true)
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

const onTogglePlanningMode = () => {
  if (!canEditCurrentWorkflowAgent.value) return
  planningMode.value = !planningMode.value
}

const onSelectedAgentChange = async agent => {
  selectedAgent.value = agent

  if (!currentWorkflowId.value || !canEditCurrentWorkflowAgent.value || !agent) {
    return
  }

  try {
    const agentConfigResult = await invokeWrapper('update_workflow_agent_id', {
      sessionId: currentWorkflowId.value,
      agentId: agent.id
    })
    const agentConfig =
      typeof agentConfigResult === 'string' ? JSON.parse(agentConfigResult) : agentConfigResult

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
    autoCompressEnabled.value = agentConfig?.autoCompress ?? true
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

const submitAskUserResponse = async content => {
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

const toggleWorkflowCompletionMute = async () => {
  await settingStore.setSetting('workflowCompletionMuted', !workflowCompletionMuted.value)
}

const handleApprovalCommand = async sessionId => {
  if (!sessionId) return
  sidebarRootFilterResetToken.value += 1
  await selectWorkflow(sessionId)
}

const getPendingApprovalTitle = item => {
  if (item?.kind === 'ask_user') {
    return t('workflow.awaitingUser')
  }
  return t('workflow.awaitingApproval')
}

const resolveInitialWorkflowId = () => {
  const savedWorkflowId = settingStore.settings.workflowLastSelectedId
  if (
    savedWorkflowId &&
    workflowStore.workflows.some(
      workflow => workflow.id === savedWorkflowId && workflow.isAutomationRun !== true
    )
  ) {
    return savedWorkflowId
  }

  return workflowStore.workflows.find(workflow => workflow.isAutomationRun !== true)?.id || null
}

watch(
  () => workflowSidebarActiveTab.value,
  async tab => {
    if (tab === 'automation') {
      const automationId =
        workflowAutomationStore.selectedAutomationId || resolveInitialAutomationId()
      if (automationId) {
        await onSelectAutomation(automationId)
      }
      return
    }

    if (tab === 'history') {
      const currentHistoryWorkflowVisible =
        currentWorkflowId.value &&
        workflowStore.workflows.some(
          workflow =>
            workflow.id === currentWorkflowId.value && workflow.isAutomationRun !== true
        )

      if (currentHistoryWorkflowVisible) {
        return
      }

      const workflowId =
        lastHistoryWorkflowId.value &&
        workflowStore.workflows.some(
          workflow =>
            workflow.id === lastHistoryWorkflowId.value && workflow.isAutomationRun !== true
        )
          ? lastHistoryWorkflowId.value
          : resolveInitialWorkflowId()

      if (workflowId) {
        await onSelectWorkflowFromHistory(workflowId)
      }
    }
  }
)

const onGlobalKeyDown = event => {
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

watch(
  () => workflowStore.displayQueueItems.length,
  (nextLength, previousLength) => {
    if (nextLength > previousLength) {
      scrollMessageListToBottom()
    }
  }
)

watch(
  () => currentWorkflowId.value,
  () => {
    visibleCompletedTaskGroupCount.value = 3
    clearImageAttachments()
  }
)

watch(
  () => canUseImageAttachments.value,
  enabled => {
    if (!enabled) {
      clearImageAttachments()
    }
  }
)

watch(
  () => agentStore.primaryAgents,
  newAgents => {
    const workflowAgentId = workflowStore.currentWorkflow?.agentId
    if (workflowAgentId) {
      const workflowAgent = newAgents.find(agent => agent.id === workflowAgentId)
      if (workflowAgent && selectedAgent.value !== workflowAgent) {
        selectedAgent.value = workflowAgent
      }
      return
    }

    const selectedAgentId = selectedAgent.value?.id
    if (selectedAgentId) {
      const remappedAgent = newAgents.find(agent => agent.id === selectedAgentId)
      if (remappedAgent && selectedAgent.value !== remappedAgent) {
        selectedAgent.value = remappedAgent
      }
      return
    }

    if (!selectedAgent.value && newAgents.length > 0) {
      selectedAgent.value = newAgents[0]
    }
  },
  { deep: true, immediate: true }
)

onMounted(async () => {
  unlistenFocusInput.value = await listen('cs://workflow-focus-input', event => {
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
  await workflowAutomationStore.fetchAutomations()
  await agentStore.fetchAgents()
  await fetchSystemSkills()
  try {
    defaultImageRecognitionPrompt.value = await invokeWrapper(
      'get_default_image_recognition_prompt'
    )
  } catch (error) {
    console.error('Failed to load default image recognition prompt:', error)
  }

  // Restore the last selected workflow if it still exists.
  const initialWorkflowId = resolveInitialWorkflowId()
  if (initialWorkflowId) {
    lastHistoryWorkflowId.value = initialWorkflowId
    await selectWorkflow(initialWorkflowId)
  } else {
    // First launch bootstrap: create one empty workflow so sending messages never hits "no session".
    await coreCreateNewWorkflow()
  }

  windowStore.initWorkflowWindowAlwaysOnTop()
  window.addEventListener('keydown', onGlobalKeyDown)
  window.addEventListener('resize', updateMaxWidth)

  // Initial scroll
  scrollMessageListToBottom()
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

.workflow-titlebar-left-actions {
  display: flex;
  align-items: center;
  gap: var(--cs-space-xs);
}

.workflow-titlebar-primary-path {
  max-width: min(40vw, 360px);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--cs-font-size-sm);
  font-weight: 500;
  color: var(--cs-text-primary);
}

.update-ready-btn {
  font-size: var(--cs-font-size);
  color: var(--cs-success-color);

  .cs {
    color: var(--cs-success-color);
  }
}
</style>
