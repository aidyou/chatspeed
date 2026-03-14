<template>
  <div class="workflow-layout">
    <titlebar>
      <template #left>
        <el-tooltip :content="$t(`chat.${sidebarCollapsed ? 'expandSidebar' : 'collapseSidebar'}`)" placement="right"
          :hide-after="0" :enterable="false">
          <div class="icon-btn upperLayer" @click="onToggleSidebar">
            <cs name="sidebar" />
          </div>
        </el-tooltip>
      </template>
      <template #center> </template>
      <template #right>
        <div class="icon-btn upperLayer pin-btn" @click="onPin" :class="{ active: isAlwaysOnTop }">
          <el-tooltip :content="$t(`common.${isAlwaysOnTop ? 'unpin' : 'pin'}`)" :hide-after="0" :enterable="false"
            placement="bottom">
            <cs name="pin" />
          </el-tooltip>
        </div>
      </template>
    </titlebar>

    <div class="workflow-main">
      <el-aside :width="sidebarWidth" :class="{ collapsed: sidebarCollapsed, dragging: isDragging }" class="sidebar"
        :style="sidebarStyle">
        <div v-show="!sidebarCollapsed" class="sidebar-tabs-container">
          <el-tabs v-model="activeSidebarTab" class="sidebar-tabs">
            <el-tab-pane :label="$t('workflow.historyTab')" name="history">
              <div class="sidebar-header upperLayer">
                <el-input v-model="searchQuery" :placeholder="$t('chat.searchChat')" :clearable="true" round>
                  <template #prefix>
                    <cs name="search" />
                  </template>
                </el-input>
              </div>
              <div class="workflow-list">
                <div class="list">
                  <div class="item" v-for="wf in filteredWorkflows" :key="wf.id" @click="selectWorkflow(wf.id)"
                    @mouseenter="hoveredWorkflowIndex = wf.id" @mouseleave="hoveredWorkflowIndex = null" :class="{
                      active: wf.id === currentWorkflowId,
                      disabled: !canSwitchWorkflow && wf.id !== currentWorkflowId
                    }">
                    <div class="workflow-title">{{ wf.title || wf.userQuery }}</div>
                    <div class="workflow-status" v-if="wf.status">
                      <span :class="['status-indicator', wf.status.toLowerCase()]"></span>
                      {{ wf.status }}
                    </div>
                    <div class="icons" v-show="wf.id === hoveredWorkflowIndex">
                      <div class="icon icon-edit" @click.stop="onEditWorkflow(wf.id)">
                        <cs name="edit" />
                      </div>
                      <div class="icon icon-delete" @click.stop="onDeleteWorkflow(wf.id)">
                        <cs name="delete" />
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </el-tab-pane>
            <el-tab-pane :label="$t('settings.agent.authorizedPaths')" name="files">
              <FileTree :paths="currentPaths" @add-path="onAddPathFromTree" @remove-path="onRemovePathFromTree" />
            </el-tab-pane>
          </el-tabs>
        </div>
      </el-aside>

      <!-- Resize Handle -->
      <div v-if="!sidebarCollapsed" class="sidebar-resize-handle" :class="{ dragging: isDragging }"
        @mousedown="onResizeStart" />

      <!-- main container -->
      <el-container class="main-container">
        <div class="messages" ref="messagesRef">
          <div v-for="(message, index) in enhancedMessages" :key="message.displayId" class="message"
            :class="[message.role, message.stepType?.toLowerCase()]">
            <div class="avatar" v-if="message.role === 'user'">
              <cs name="talk" class="user-icon" />
            </div>
            <div class="content-container">
              <div class="content" v-if="message.role === 'user'">
                <pre class="simple-text">{{ message.message }}</pre>
              </div>
              <div v-else class="ai-content chat">
                <!-- CLI Style Tool Call (Results) -->
                <div v-if="message.role === 'tool'" class="cli-tool-call"
                  :class="[message.toolDisplay.toolType || 'tool-system', message.toolDisplay.isError ? 'status-error' : 'status-success']">

                  <!-- finish_task special display -->
                  <template
                    v-if="message.toolDisplay?.action === t('workflow.finishTask') || message.toolDisplay?.action?.includes('Finish')">
                    <div class="tool-line finish-task-display">
                      <cs :name="message.toolDisplay.isError ? 'check-x' : 'check-circle'" size="14px"
                        class="tool-type-icon finish-icon" />
                      <span class="finish-text">{{ t('workflow.finishTask') }}</span>
                    </div>
                  </template>

                  <!-- Normal tool call display -->
                  <template v-else>
                    <div class="tool-line title-wrap expandable" :class="{ 'tool-rejected': message.isRejected }"
                      @click="toggleMessageExpand(message.displayId)">
                      <cs :name="message.toolDisplay.icon || 'tool'" size="14px" class="tool-type-icon" />
                      <span class="tool-name">{{ message.toolDisplay.action }}</span>
                      <span class="tool-target">{{ message.toolDisplay.target }}</span>
                      <cs v-if="message.isApproved" name="check" size="14px" class="approved-icon" />
                    </div>
                    <!-- Hide summary when expanded -->
                    <div class="tool-line summary expandable" v-if="!isMessageExpanded(message)"
                      @click="toggleMessageExpand(message.displayId)">
                      <span class="corner-icon">⎿</span>
                      <span class="summary-text">{{ message.toolDisplay.summary }}</span>
                      <span class="expand-hint">(click to expand)</span>
                    </div>
                    <div v-if="isMessageExpanded(message)" class="tool-detail">
                      <MarkdownSimple v-if="message.toolDisplay.displayType === 'diff'"
                        :content="getDiffMarkdown(removeSystemReminder(message.message))" />
                      <div v-else-if="message.toolDisplay.displayType === 'choice'" class="choice-container">
                        <div class="choice-question">{{
                          parseChoiceContent(removeSystemReminder(message.message)).question
                        }}
                        </div>
                        <div class="choice-options">
                          <el-button v-for="opt in parseChoiceContent(removeSystemReminder(message.message)).options"
                            :key="opt" size="small" plain round :disabled="isRunning" @click="sendUserChoice(opt)">
                            {{ opt }}
                          </el-button>
                        </div>
                      </div>
                      <pre v-else class="raw-content">{{ removeSystemReminder(message.message) }}</pre>
                    </div>
                  </template>
                </div>

                <!-- Regular Assistant Content -->
                <div v-else>
                  <!-- Thought/Content FIRST (Separate reasoning field has priority) -->
                  <div v-if="message.reasoning || message.stepType === 'Think'" class="reasoning-container">
                    <div class="reasoning-header" @click="toggleReasoningExpand(message.displayId)">
                      <cs name="reasoning" size="14px" class="reasoning-icon"
                        :class="{ rotating: isRunning && !getParsedMessage(message).content && (message.metadata?.tool_calls?.length || 0) === 0 && !isReasoningExpanded(message.displayId) && message === lastAssistantMessage }" />
                      <span class="reasoning-text" :class="{ expanded: isReasoningExpanded(message.displayId) }">
                        <template v-if="isReasoningExpanded(message.displayId)">
                          {{ t('workflow.thinkingExpanded') || 'Thinking Process' }}
                        </template>
                        <template
                          v-else-if="isRunning && !getParsedMessage(message).content && (message.metadata?.tool_calls?.length || 0) === 0 && message === lastAssistantMessage">
                          {{ getReasoningPreview(message.reasoning || message.message) }}
                        </template>
                        <template v-else>
                          {{ t('workflow.thoughtCompleted') || 'Thought Complete' }}
                        </template>
                      </span>
                      <span class="reasoning-toggle">
                        {{ isReasoningExpanded(message.displayId) ? '▲' : '▼' }}
                      </span>
                    </div>
                    <div v-if="isReasoningExpanded(message.displayId)" class="reasoning-content">
                      {{ message.reasoning || message.message }}
                    </div>
                  </div>
                  <MarkdownSimple v-if="getParsedMessage(message).content"
                    :content="getParsedMessage(message).content" />

                  <!-- Tool Call Indicators SECOND (Only pending ones) -->
                  <div v-if="message.pendingToolCalls?.length > 0" class="cli-tool-calls-container">
                    <div v-for="call in message.pendingToolCalls" :key="call.id" class="cli-tool-call pending"
                      :class="[call.toolType || 'tool-system', 'status-running']">
                      <div class="tool-line title-wrap">
                        <cs :name="call.icon || 'tool'" size="14px" class="tool-type-icon" />
                        <span class="tool-name">{{ call.action }}</span>
                        <span class="tool-target">{{ call.target }}</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div v-if="isChatting && (chatState.content || chatState.reasoning)" class="message assistant chatting">
            <div class="content-container">
              <div class="ai-content chat">
                <div v-if="chatState.reasoning" class="reasoning-container">
                  <div class="reasoning-header">
                    <cs name="reasoning" size="14px" class="reasoning-icon" :class="{ rotating: !chatState.content }" />
                    <span class="reasoning-text">
                      {{ chatState.content ? (t('workflow.thoughtCompleted') || 'Thought Complete') :
                        getReasoningPreview(chatState.reasoning) }}
                    </span>
                  </div>
                </div>
                <!-- Streaming Blocks (Optimized rendering) -->
                <div v-for="(block, bIdx) in chatState.blocks" :key="bIdx">
                  <!-- Output all blocks from the parser (paragraph, code, math, etc.) -->
                  <MarkdownSimple :content="block.content" />
                </div>

                <!-- Retry Countdown... -->
                <div v-if="chatState.retryInfo && chatState.retryInfo.nextRetryIn > 0" class="retry-status-alert">
                  <el-alert type="warning" :closable="false" show-icon>
                    <template #title>
                      {{ $t('workflow.retrying', {
                        attempt: chatState.retryInfo.attempt,
                        total: chatState.retryInfo.total,
                        seconds: chatState.retryInfo.nextRetryIn
                      }) }}
                    </template>
                  </el-alert>
                </div>
              </div>
            </div>
          </div>

          <!-- Context Compression Status -->
          <div v-if="isCompressing" class="compression-status">
            <div class="compression-indicator">
              <cs name="loading" size="14px" class="rotating" />
              <span class="compression-text">{{ compressionMessage }}</span>
            </div>
          </div>
        </div>

        <!-- Status Panel (Floating) -->
        <StatusPanel />

        <!-- footer -->
        <el-footer class="input-container">
          <!-- Slash Command Suggestion Panel -->
          <div v-if="showSkillSuggestions && filteredSystemSkills.length > 0" class="slash-command-panel">
            <div v-for="(skill, idx) in filteredSystemSkills" :key="skill.name" class="command-item"
              :class="{ active: idx === selectedSkillIndex }" @click="onSkillSelect(skill)">
              <div class="command-name">/{{ skill.name }}</div>
              <div class="command-desc">{{ skill.description }}</div>
            </div>
          </div>

          <!-- File At-mention Suggestion Panel -->
          <div v-if="showFileSuggestions && fileSuggestions.length > 0"
            class="slash-command-panel file-suggestion-panel compact">
            <div v-for="(file, idx) in fileSuggestions" :key="file.path" class="command-item"
              :class="{ active: idx === selectedFileIndex }" @click="onFileSelect(file)">
              <cs :name="file.is_directory ? 'folder' : 'file'" size="14px" class="file-icon" />
              <span class="file-path">{{ file.relative_path }}</span>
            </div>
          </div>

          <div class="input">
            <div v-if="currentWorkflow?.status === 'paused'" class="input-status-hint">
              <div class="hint-header">
                <cs name="talk" size="12px" />
                <span>{{ activeAskUser ? activeAskUser.question : 'AI is waiting for your response...' }}</span>
              </div>
              <div v-if="activeAskUser" class="hint-options">
                <el-button v-for="opt in activeAskUser.options" :key="opt" size="small" plain round
                  @click="inputMessage = opt">
                  {{ opt }}
                </el-button>
              </div>
            </div>
            <StatusNotifier />
            <div class="input-header" v-if="!currentWorkflowId">
              <div class="model-selector-trigger" @click="openModelSelector">
                <span class="model-name">{{ activeModelName }} ({{ planningMode ? 'plan' : 'act' }})</span>
                <cs name="arrow-down" size="12px" />
              </div>
            </div>
            <el-input ref="inputRef" v-model="inputMessage" type="textarea" :autosize="{ minRows: 1, maxRows: 10 }"
              :placeholder="$t('chat.inputMessagePlaceholder', { at: '/' })" @keydown="onInputKeyDown"
              @compositionstart="onCompositionStart" @compositionend="onCompositionEnd" />

            <div class="input-footer">
              <div class="footer-left">
                <div class="agent-selector-wrap" :class="{ disabled: currentWorkflowId }">
                  <AgentSelector v-model="selectedAgent" :agent="currentWorkflow?.agentId
                    ? agentStore.agents.find(a => a.id === currentWorkflow.agentId)
                    : null
                    " :disabled="!!currentWorkflowId" />
                </div>

                <!-- Authorized Paths removed - now only in sidebar tab -->

                <div class="icons">
                  <el-tooltip :content="$t('workflow.planningModeTooltip')" placement="top">
                    <label class="icon-btn upperLayer" :class="{ active: planningMode }"
                      @click="planningMode = !planningMode">
                      <cs name="skill-plan" class="small" />
                    </label>
                  </el-tooltip>

                  <!-- Final Audit Toggle -->
                  <el-tooltip :content="$t('workflow.finalAuditTooltip')" placement="top">
                    <label class="final-audit-toggle icon-btn upperLayer" :class="finalAuditMode"
                      @click="toggleFinalAuditMode">
                      <cs name="check-circle" class="small" />
                      <span class="audit-label" v-if="finalAuditMode !== 'off'">{{ finalAuditMode.toUpperCase()
                        }}</span>
                    </label>
                  </el-tooltip>

                  <!-- Approval Level Dropdown -->
                  <el-dropdown trigger="click" @command="val => approvalLevel = val">
                    <label class="icon-btn upperLayer" :class="{ 'warning-mode': approvalLevel === 'full' }">
                      <cs
                        :name="approvalLevel === 'default' ? 'setting' : (approvalLevel === 'smart' ? 'brain' : 'warning')"
                        class="small" />
                    </label>
                    <template #dropdown>
                      <el-dropdown-menu class="approval-level-dropdown">
                        <el-dropdown-item command="default" :class="{ active: approvalLevel === 'default' }">
                          <cs name="setting" size="14px" class="dropdown-icon" />
                          <span class="dropdown-text">{{ $t('settings.agent.approvalLevelDefault') }}</span>
                          <cs v-if="approvalLevel === 'default'" name="check" size="14px" class="dropdown-check" />
                        </el-dropdown-item>
                        <el-dropdown-item command="smart" :class="{ active: approvalLevel === 'smart' }">
                          <cs name="brain" size="14px" class="dropdown-icon" />
                          <span class="dropdown-text">{{ $t('settings.agent.approvalLevelSmart') }}</span>
                          <cs v-if="approvalLevel === 'smart'" name="check" size="14px" class="dropdown-check" />
                        </el-dropdown-item>
                        <el-dropdown-item command="full" class="danger-option"
                          :class="{ active: approvalLevel === 'full' }">
                          <cs name="warning" size="14px" class="dropdown-icon" />
                          <span class="dropdown-text">{{ $t('settings.agent.approvalLevelFull') }}</span>
                          <cs v-if="approvalLevel === 'full'" name="check" size="14px" class="dropdown-check" />
                        </el-dropdown-item>
                      </el-dropdown-menu>
                    </template>
                  </el-dropdown>

                  <el-tooltip :content="$t('workflow.newWorkflow')" :hide-after="0" :enterable="false" placement="top">
                    <label @click="createNewWorkflow" :class="{ disabled: isRunning }">
                      <cs name="new-chat" class="small" :class="{ disabled: isRunning }" />
                    </label>
                  </el-tooltip>
                </div>
              </div>
              <div class="icons">
                <el-button v-if="isAwaitingApproval" size="small" round type="success" @click="onApprovePlan">
                  {{ $t('workflow.approvePlan') }}
                </el-button>
                <el-button
                  v-if="!isRunning && !isAwaitingApproval && currentWorkflowId && currentWorkflow?.status !== 'completed' && currentWorkflow?.status !== 'error'"
                  size="small" round type="primary" @click="onContinue">
                  {{ $t('workflow.continue') }}
                </el-button>
                <cs name="stop" @click="onStop" v-if="isRunning" />
                <cs name="send" @click="onSendMessage" :class="{ disabled: !canSendMessage }" />
              </div>
            </div>
          </div>
        </el-footer>
      </el-container>
    </div>

    <!-- edit workflow dialog -->
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
      :loading="approvalLoading" @approve="onApproveAction" @approveAll="onApproveAllAction" @reject="onRejectAction" />

    <WorkflowModelSelector v-model="modelSelectorVisible" :initial-tab="modelSelectorTab" :agent="selectedAgent"
      @save="onModelConfigSave" />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, onUnmounted, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { open } from '@tauri-apps/plugin-dialog'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'

import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { useSettingStore } from '@/stores/setting'
import { useWindowStore } from '@/stores/window'

import Titlebar from '@/components/window/Titlebar.vue'
import MarkdownSimple from '@/components/workflow/MarkdownSimple.vue'
import AgentSelector from '@/components/workflow/AgentSelector.vue'
import StatusPanel from '@/components/workflow/StatusPanel.vue'
import StatusNotifier from '@/components/workflow/StatusNotifier.vue'
import ApprovalDialog from '@/components/workflow/ApprovalDialog.vue'
import FileTree from '@/components/workflow/FileTree.vue'
import WorkflowModelSelector from '@/components/workflow/WorkflowModelSelector.vue'

// Import types
import { MarkdownStreamParser } from '@/libs/markdown-stream-parser'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const settingStore = useSettingStore()
const windowStore = useWindowStore()

const unlistenFocusInput = ref(null)
const unlistenWorkflowEvents = ref(null)
const osType = ref('') // To store OS type from backend
const hoveredWorkflowIndex = ref(null) // For workflow hover effects

// approval dialog
const approvalVisible = ref(false)
const approvalAction = ref('')
const approvalDetails = ref('')
const approvalRequestId = ref('')
const approvalLoading = ref(false)

// Chatting state for real-time streaming
const chattingParser = new MarkdownStreamParser()
const isChatting = computed(() => workflowStore.isRunning)
const chatState = ref({
  content: '',
  reasoning: '',
  blocks: []
})

// Context compression state
const isCompressing = ref(false)
const compressionMessage = ref('')

// edit workflow dialog
const editWorkflowDialogVisible = ref(false)
const editWorkflowId = ref(null)
const editWorkflowTitle = ref('')

const sidebarCollapsed = ref(!windowStore.workflowSidebarShow)
const sidebarWidthValue = ref(300) // Default sidebar width
const sidebarWidth = computed(() => sidebarCollapsed.value ? '0px' : `${sidebarWidthValue.value}px`)
const sidebarStyle = computed(() => ({
  '--sidebar-width': sidebarCollapsed.value ? '0px' : `${sidebarWidthValue.value}px`
}))

// Resize dragging state
const isDragging = ref(false)
const maxSidebarWidth = ref(window.innerWidth * 0.5)

// Update max width on window resize
const updateMaxWidth = () => {
  maxSidebarWidth.value = window.innerWidth * 0.5
}

// Resize handlers
const onResizeStart = (e) => {
  if (sidebarCollapsed.value) return
  isDragging.value = true
  e.preventDefault()

  const startX = e.clientX
  const startWidth = sidebarWidthValue.value

  const onMouseMove = (moveEvent) => {
    const delta = moveEvent.clientX - startX
    const newWidth = Math.max(200, Math.min(startWidth + delta, maxSidebarWidth.value))
    sidebarWidthValue.value = newWidth
  }

  const onMouseUp = () => {
    isDragging.value = false
    document.removeEventListener('mousemove', onMouseMove)
    document.removeEventListener('mouseup', onMouseUp)
  }

  document.addEventListener('mousemove', onMouseMove)
  document.addEventListener('mouseup', onMouseUp)
}

const activeSidebarTab = ref('history')
const searchQuery = ref('')
const inputMessage = ref('')
const selectedAgent = ref(null)
const approvalLevel = ref('default') // 'default', 'smart', 'full'
const finalAuditMode = ref('on') // 'agent', 'on', 'off'
const planningMode = ref(false)

const activeModelName = computed(() => {
  // 1. Try to get from current configs (reflected in settings/workflow)
  const tab = planningMode.value ? 'plan' : 'act'
  const workflow = workflowStore.currentWorkflow || (workflowStore.workflows.length > 0 ? workflowStore.workflows[0] : null)

  let providerId = null
  let modelId = null

  if (workflow && workflow.agentConfig && workflow.agentConfig.models) {
    const models = workflow.agentConfig.models
    const model = planningMode.value ? (models.plan || models.act) : models.act
    if (model) {
      providerId = model.id
      modelId = model.model
    }
  }

  if (providerId && modelId) {
    const provider = modelStore.getModelProviderById(providerId)
    if (provider) {
      const model = provider.models.find(m => m.id === modelId)
      if (model) return model.name
    }
    return modelId
  }

  if (selectedAgent.value) return selectedAgent.value.name
  return 'Select Model'
})

const onModelConfigSave = async (configs) => {
  console.log('Saving model config:', configs)
  try {
    // 1. Save to current agent if selected
    if (selectedAgent.value) {
      const updatedAgent = { ...selectedAgent.value }
      const modelsObj = {
        plan: configs.plan,
        act: configs.act
      }
      updatedAgent.models = JSON.stringify(modelsObj)

      // Update local store and persist to DB
      await agentStore.saveAgent(updatedAgent)
      // Refetch to sync state
      await agentStore.fetchAgents()
      // Re-select current to trigger reactivity
      selectedAgent.value = agentStore.agents.find(a => a.id === updatedAgent.id) || updatedAgent
    }

    // 2. If we have an active workflow session, signal the engine
    if (currentWorkflowId.value) {
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal: JSON.stringify({
          type: 'update_model_config',
          configs: configs
        })
      })

      // Refresh current workflow state from DB to update UI
      await workflowStore.selectWorkflow(currentWorkflowId.value)
    }
    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    console.error('Failed to save model config:', error)
    showMessage(t('common.saveFailed'), 'error')
  }
}
const composing = ref(false)
const compositionJustEnded = ref(false)
const messagesRef = ref(null)
const inputRef = ref(null)

// Retry timer reference for memory safety
let retryCountdownTimer = null

const clearRetryTimer = () => {
  if (retryCountdownTimer) {
    clearInterval(retryCountdownTimer)
    retryCountdownTimer = null
  }
}

onUnmounted(() => {
  clearRetryTimer()
})

// System Skills (from ~/.chatspeed/skills etc) slash command logic
const systemSkills = ref([])
const builtinCommands = [
  { name: 'settings', description: 'Open settings window' },
  { name: 'models', description: 'Open model selection window' },
  { name: 'mcp', description: 'Open MCP settings' },
  { name: 'proxy', description: 'Open proxy settings' },
  { name: 'agent', description: 'Open agent settings' },
  { name: 'about', description: 'Open about page' }
]
const showSkillSuggestions = ref(false)
const selectedSkillIndex = ref(0)
const filteredSystemSkills = computed(() => {
  // Only search if starts with /
  if (!inputMessage.value.startsWith('/')) return []
  const query = inputMessage.value.substring(1).toLowerCase()

  const skills = systemSkills.value.map(s => ({ name: s.name, description: s.description, type: 'skill' }))
  const commands = builtinCommands.map(c => ({ name: c.name, description: c.description, type: 'command' }))

  return [...commands, ...skills]
    .filter(item =>
      item.name.toLowerCase().includes(query) ||
      (item.description && item.description.toLowerCase().includes(query))
    )
    .sort((a, b) => {
      const aName = a.name.toLowerCase()
      const bName = b.name.toLowerCase()
      
      // 1. Prioritize exact name match
      if (aName === query && bName !== query) return -1
      if (aName !== query && bName === query) return 1

      // 2. Prioritize "starts with" name match
      const aStarts = aName.startsWith(query)
      const bStarts = bName.startsWith(query)
      if (aStarts && !bStarts) return -1
      if (!aStarts && bStarts) return 1

      // 3. Prioritize "includes" name match
      const aIncludes = aName.includes(query)
      const bIncludes = bName.includes(query)
      if (aIncludes && !bIncludes) return -1
      if (!aIncludes && bIncludes) return 1

      // 4. Fallback to alphabetical order
      return aName.localeCompare(bName)
    })
})

const modelSelectorVisible = ref(false)
const modelSelectorTab = ref('act') // 'plan' or 'act'

const openModelSelector = () => {
  modelSelectorTab.value = planningMode.value ? 'plan' : 'act'
  modelSelectorVisible.value = true
}
const modelSelectorMode = ref('provider') // 'provider' or 'proxy'

// File At-mention logic
const showFileSuggestions = ref(false)
const selectedFileIndex = ref(0)
const fileSuggestions = ref([])
const fileQuery = ref('')
const ignoreNextSearch = ref(false)

const searchFiles = async (query) => {
  if (ignoreNextSearch.value) return
  if (!currentPaths.value || currentPaths.value.length === 0) return
  try {
    const results = await invokeWrapper('search_workspace_files', {
      paths: currentPaths.value,
      query: query
    })
    fileSuggestions.value = results || []
    showFileSuggestions.value = fileSuggestions.value.length > 0
    selectedFileIndex.value = 0
  } catch (error) {
    console.error('Failed to search files:', error)
  }
}

const onFileSelect = (file) => {
  ignoreNextSearch.value = true
  const cursorPosition = inputRef.value?.$el.querySelector('textarea').selectionStart || 0
  const textBeforeCursor = inputMessage.value.slice(0, cursorPosition)
  const textAfterCursor = inputMessage.value.slice(cursorPosition)

  // Replace the @query part with @path
  const newTextBefore = textBeforeCursor.replace(/@([^\s]*)$/, `@${file.relative_path} `)
  inputMessage.value = newTextBefore + textAfterCursor

  showFileSuggestions.value = false
  selectedFileIndex.value = 0

  nextTick(() => {
    if (inputRef.value) {
      inputRef.value.focus()
      const newPos = newTextBefore.length
      const textarea = inputRef.value?.$el.querySelector('textarea')
      if (textarea) {
        textarea.setSelectionRange(newPos, newPos)
      }
    }
    // Allow search again after UI has updated
    setTimeout(() => {
      ignoreNextSearch.value = false
    }, 100)
  })
}

const fetchSystemSkills = async () => {
  try {
    const result = await invokeWrapper('get_system_skills')
    systemSkills.value = result || []
  } catch (error) {
    console.error('Failed to fetch system skills:', error)
  }
}
const onSkillSelect = (skill) => {
  // Replace the slash command with the full skill command
  inputMessage.value = '/' + skill.name + (skill.type === 'command' ? '' : ' ')
  showSkillSuggestions.value = false
  selectedSkillIndex.value = 0

  // If it's a builtin command (UI action), we execute it immediately
  if (skill.type === 'command') {
    onSendMessage()
  } else {
    // For skills (AI logic), we focus and let user add more details (e.g., commit message)
    nextTick(() => {
      if (inputRef.value) {
        inputRef.value.focus()
      }
    })
  }
}
watch(approvalLevel, async (newVal) => {
  if (currentWorkflowId.value) {
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal: JSON.stringify({
        type: 'update_approval_level',
        level: newVal
      })
    })
    // Refresh to sync local state if needed
    await workflowStore.selectWorkflow(currentWorkflowId.value)
  }
})

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

watch(inputMessage, (newVal) => {
  // TRIGGERS ONLY if '/' is the very first character of the whole input
  if (newVal === '/') {
    showSkillSuggestions.value = systemSkills.value.length > 0
    selectedSkillIndex.value = 0
  } else if (!newVal.startsWith('/') || newVal === '') {
    showSkillSuggestions.value = false
  }

  // At-mention detection
  if (inputRef.value) {
    const cursorPosition = inputRef.value?.$el.querySelector('textarea').selectionStart || 0
    const textBeforeCursor = newVal.slice(0, cursorPosition)
    const match = textBeforeCursor.match(/@([^\s]*)$/)
    if (match) {
      searchFiles(match[1])
    } else {
      showFileSuggestions.value = false
    }
  }
})

watch(selectedAgent, (newAgent, oldAgent) => {
  // Only clear pending paths if the agent ID actually changed to a different one
  if (newAgent && oldAgent && newAgent.id !== oldAgent.id) {
    pendingPaths.value = []
  }
})

// Authorized paths management
const allowedPaths = computed(() => {
  const paths = currentWorkflow.value?.allowedPaths
  console.log('currentWorkflow:', currentWorkflow.value)
  if (!paths) return []
  try {
    const parsed = typeof paths === 'string' ? JSON.parse(paths) : paths
    console.log('Workflow.vue: parsed allowedPaths:', parsed)
    return parsed
  } catch (e) {
    return []
  }
})

// Pending paths for new workflow (cached locally until workflow is created)
const pendingPaths = ref([])

// Current paths: use workflow paths if available, pending paths for new workflow, or agent paths as default
const currentPaths = computed(() => {
  if (currentWorkflowId.value) {
    return allowedPaths.value
  }
  // No workflow - use pending paths if any, otherwise show agent's paths as reference
  if (pendingPaths.value.length > 0) {
    return pendingPaths.value
  }
  // Show agent's default paths as reference (read-only display)
  if (!selectedAgent.value) return []
  try {
    const paths = selectedAgent.value.allowedPaths
    if (!paths) return []
    return typeof paths === 'string' ? JSON.parse(paths) : paths
  } catch (e) {
    return []
  }
})

// Can edit paths if we have a workflow, or if we have a selected agent (for new workflow)
const canEditPaths = computed(() => {
  return !!currentWorkflowId.value || !!selectedAgent.value
})

const showPathsDisabledMessage = () => {
  showMessage(t('workflow.selectAgentFirst'), 'warning')
}

const displayAllowedPath = computed(() => {
  const paths = currentPaths.value
  console.log('Workflow.vue: computing displayAllowedPath for:', paths)
  if (!paths || paths.length === 0) return t('settings.agent.workingDirectory')
  const firstPath = paths[0]
  if (!firstPath) return t('settings.agent.workingDirectory')
  // Try to get last segment of path
  const parts = firstPath.split(/[/\\]/).filter(p => p !== '')
  const result = parts[parts.length - 1] || firstPath
  console.log('Workflow.vue: displayAllowedPath result:', result)
  return result
})

const onAddPath = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.agent.selectDirectory')
    })
    if (selected) {
      if (currentWorkflowId.value) {
        // Editing existing workflow
        const newPaths = [...allowedPaths.value]
        if (!newPaths.includes(selected)) {
          newPaths.push(selected)
          await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
          // Immediately notify executor to update path_guard in memory
          await invokeWrapper('workflow_signal', {
            sessionId: currentWorkflowId.value,
            signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
          })
        }
      } else {
        // No workflow yet - cache in pendingPaths
        if (!pendingPaths.value.includes(selected)) {
          pendingPaths.value.push(selected)
        }
      }
    }
  } catch (error) {
    console.error('Failed to add path:', error)
  }
}

const onRemovePath = async (index) => {
  if (currentWorkflowId.value) {
    // Editing existing workflow
    const newPaths = [...allowedPaths.value]
    newPaths.splice(index, 1)
    await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
    // Immediately notify executor
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
    })
  } else {
    // No workflow yet - remove from pendingPaths
    pendingPaths.value.splice(index, 1)
  }
}

// Handle add path from FileTree component
const onAddPathFromTree = async (selected) => {
  if (!selected) return
  if (currentWorkflowId.value) {
    // Editing existing workflow
    const newPaths = [...allowedPaths.value]
    if (!newPaths.includes(selected)) {
      newPaths.push(selected)
      await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
      // Immediately notify executor to update path_guard in memory
      await invokeWrapper('workflow_signal', {
        sessionId: currentWorkflowId.value,
        signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
      })
    }
  } else {
    // No workflow yet - cache in pendingPaths
    if (!pendingPaths.value.includes(selected)) {
      pendingPaths.value.push(selected)
    }
  }
}

// Handle remove path from FileTree component
const onRemovePathFromTree = async (path) => {
  if (!path) return
  if (currentWorkflowId.value) {
    // Editing existing workflow
    const newPaths = allowedPaths.value.filter(p => p !== path)
    await workflowStore.updateWorkflowAllowedPaths(currentWorkflowId.value, newPaths)
    // Immediately notify executor
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal: JSON.stringify({ type: 'update_allowed_paths', paths: newPaths })
    })
  } else {
    // No workflow yet - remove from pendingPaths
    const index = pendingPaths.value.indexOf(path)
    if (index > -1) {
      pendingPaths.value.splice(index, 1)
    }
  }
}

// Message expansion state
const expandedMessages = ref(new Set())
const toggleMessageExpand = (id) => {
  if (expandedMessages.value.has(id)) {
    expandedMessages.value.delete(id)
  } else {
    expandedMessages.value.add(id)
  }
}
const isMessageExpanded = (message) => {
  // Only force expansion for 'Ask User' to ensure visibility of interaction points.
  // Everything else (especially heavy Diffs) should be collapsed by default.
  if (message.toolDisplay?.action === 'Ask User') return true
  return expandedMessages.value.has(message.displayId)
}

// Reasoning expansion state
const expandedReasonings = ref(new Set())
const toggleReasoningExpand = (id) => {
  if (expandedReasonings.value.has(id)) {
    expandedReasonings.value.delete(id)
  } else {
    expandedReasonings.value.add(id)
  }
}
const isReasoningExpanded = (id) => expandedReasonings.value.has(id)

// Get last sentence from text (split by punctuation)
const getLastSentence = (text) => {
  if (!text) return ''
  const sentences = text.split(/(?<=[。！？.!?])\s*/).filter(s => s.trim())
  return sentences[sentences.length - 1] || text.slice(-50)
}

// Get preview text for reasoning (last sentence with max length)
const getReasoningPreview = (text, maxLen = 50) => {
  if (!text) return t('workflow.thinking') || 'Thinking...'
  const last = getLastSentence(text)
  if (last.length <= maxLen) return last
  return last.slice(0, maxLen) + '...'
}

// Compute last assistant message for streaming state detection
const lastAssistantMessage = computed(() => {
  return enhancedMessages.value
    .filter(m => m.role === 'assistant')
    .pop()
})

// Active Ask User question and options for the input area
const activeAskUser = computed(() => {
  if (currentWorkflow.value?.status !== 'paused') return null
  // Look for the very last tool message which is an Ask User
  const lastMsg = enhancedMessages.value[enhancedMessages.value.length - 1]
  if (lastMsg?.role === 'tool' && lastMsg.toolDisplay?.action === 'Ask User' && lastMsg.toolDisplay?.displayType === 'choice') {
    return parseChoiceContent(removeSystemReminder(lastMsg.message))
  }
  return null
})

// Helper functions for truncating text (UTF-8 safe)
const truncateUrl = (url, maxLen = 40) => {
  if (!url || url.length <= maxLen) return url
  const keep = Math.floor((maxLen - 3) / 2)
  return url.slice(0, keep) + '...' + url.slice(-keep)
}

// UTF-8 safe truncation using Array.from to properly handle multibyte characters
const truncateText = (text, maxLen = 25) => {
  if (!text) return ''
  const chars = Array.from(text)
  if (chars.length <= maxLen) return text
  return chars.slice(0, maxLen - 3).join('') + '...'
}

const truncatePath = (path, maxLen = 30) => {
  if (!path || path.length <= maxLen) return path
  // For paths, try to keep the filename and truncate the middle
  const parts = path.split('/')
  const fileName = parts.pop()
  if (fileName && fileName.length > maxLen - 10) {
    return '.../' + truncateText(fileName, maxLen - 4)
  }
  const dir = parts.join('/')
  const available = maxLen - fileName.length - 4 // 4 for ".../"
  if (available > 5) {
    return truncateText(dir, available) + '/.../' + fileName
  }
  return '.../' + fileName
}

// Format tool title with icon, tool type class, and display text
// Returns action (verb) and target separately for proper display in template
const formatToolTitle = (name, args) => {
  // Helper to extract domain from URL
  const getDomain = (url) => {
    if (!url) return ''
    try {
      const urlObj = new URL(url.startsWith('http') ? url : `https://${url}`)
      return urlObj.hostname
    } catch (e) {
      return url
    }
  }

  const toolFormatters = {
    'read_file': (args) => {
      const path = args.file_path || args.path || ''
      const limit = args.limit
      const offset = args.offset
      let suffix = ''
      if (limit !== undefined && offset !== undefined) {
        suffix = ` L${limit}-${offset}`
      } else if (limit !== undefined) {
        suffix = ` L${limit}`
      } else if (offset !== undefined) {
        suffix = ` @${offset}`
      }
      return { icon: 'file', toolType: 'tool-file', action: 'Read', target: `${path}${suffix}` }
    },

    'write_file': (args) => {
      const path = args.file_path || args.path || ''
      return { icon: 'file', toolType: 'tool-file', action: 'Write', target: path }
    },

    'edit_file': (args) => {
      const path = args.file_path || args.path || ''
      return { icon: 'edit', toolType: 'tool-file', action: `Edit ${path}`, target: '' }
    },

    'list_dir': (args) => {
      const path = args.path || args.dir || '.'
      return { icon: 'folder', toolType: 'tool-file', action: 'List', target: path }
    },

    'glob': (args) => {
      const pattern = args.pattern || args.glob || ''
      return { icon: 'search', toolType: 'tool-file', action: `Glob ${pattern}`, target: '' }
    },

    'grep': (args) => {
      const pattern = args.pattern || args.query || ''
      const path = args.path || ''
      const action = path ? `Grep "${pattern}" in ${path}` : `Grep "${pattern}"`
      return { icon: 'search', toolType: 'tool-file', action, target: '' }
    },

    'web_fetch': (args) => {
      const url = args.url || ''
      return { icon: 'link', toolType: 'tool-network', action: `Fetch ${url}`, target: '' }
    },

    'web_search': (args) => {
      const query = args.query || ''
      const numResults = args.num_results
      const action = numResults !== undefined ? `Search "${query}" (Count: ${numResults})` : `Search "${query}"`
      return { icon: 'search', toolType: 'tool-network', action, target: '' }
    },

    'bash': (args) => {
      const cmd = args.command || ''
      return { icon: 'terminal', toolType: 'tool-system', action: `Bash: ${truncateText(cmd, 60)}`, target: '' }
    },

    'todo_create': (args) => {
      // Handle single todo creation
      const subject = args.subject || args.title || ''
      if (subject) {
        return { icon: 'add', toolType: 'tool-todo', action: t('workflow.todo.create'), target: truncateText(subject, 25) }
      }
      // Handle batch creation
      const tasks = args.tasks
      if (tasks && Array.isArray(tasks)) {
        const taskList = tasks.map(t => `[ ] ${truncateText(t.subject || t.title || '', 20)}`).join('\\n')
        return { icon: 'add', toolType: 'tool-todo', action: t('workflow.todo.createBatch'), target: `${tasks.length}项` }
      }
      return { icon: 'add', toolType: 'tool-todo', action: t('workflow.todo.create'), target: '' }
    },

    'todo_update': (args) => {
      const subject = args.subject || args.title || ''
      const status = args.status || ''
      let statusText = ''
      if (status === 'completed') statusText = t('workflow.todo.statusCompleted')
      else if (status === 'in_progress') statusText = t('workflow.todo.statusInProgress')
      else if (status === 'pending') statusText = t('workflow.todo.statusPending')
      else statusText = status

      if (subject && statusText) {
        return { icon: 'check', toolType: 'tool-todo', action: `Update ${truncateText(subject, 20)} to ${statusText}`, target: '' }
      }
      return { icon: 'check', toolType: 'tool-todo', action: t('workflow.todo.update'), target: '' }
    },
    'todo_list': () => ({ icon: 'list', toolType: 'tool-todo', action: t('workflow.todo.list'), target: '' }),
    'todo_get': () => ({ icon: 'list', toolType: 'tool-todo', action: t('workflow.todo.view'), target: '' }),
    'finish_task': () => ({ icon: 'check-circle', toolType: 'tool-todo', action: t('workflow.finishTask'), target: '' })
  }

  const formatter = toolFormatters[name]
  if (formatter) {
    return formatter(args || {})
  }

  // Default handling - just show the tool name
  const defaultName = name.replace(/_/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
  return { icon: 'tool', toolType: 'tool-system', action: defaultName, target: '' }
}

// Standardize tool display info from metadata
const getToolDisplayInfo = (message) => {
  const meta = message.metadata || {}
  const isError = message.isError || message.is_error || meta.is_error || false

  // 1. Try to extract tool call info
  const toolCall = meta.tool_call || {}
  const func = toolCall.function || toolCall
  const name = func.name || ''
  const rawArgs = func.arguments || func.input || {}

  let args = rawArgs
  if (typeof rawArgs === 'string') {
    try {
      args = JSON.parse(rawArgs)
    } catch (e) {
      args = {}
    }
  }

  // 2. Format using standard rules
  const formatted = formatToolTitle(name, args)

  // 3. Robust Priority:
  // If backend provided a title explicitly, use it as the main action.
  // This is crucial for results (observations) where original tool_call might be obscured.
  let finalAction = formatted.action
  let finalTarget = formatted.target
  let finalIcon = formatted.icon
  let finalToolType = formatted.toolType

  if (meta.title && meta.title.trim()) {
    finalAction = removeSystemReminder(meta.title)
    finalTarget = '' // Target is usually embedded in the title
  }

  // Fallback for missing action (prevents empty titles)
  if (!finalAction && !name) {
    // If it's a tool result but we lost the name, use a generic "Result"
    finalAction = t('chat.toolResult') || 'Result'
  }

  return {
    title: finalAction + (finalTarget ? ` ${finalTarget}` : ''),
    summary: removeSystemReminder(meta.summary || (isError ? 'Failed' : 'Executing...')),
    isError: isError,
    displayType: meta.display_type || 'text',
    icon: finalIcon,
    toolType: finalToolType,
    action: finalAction,
    target: finalTarget
  }
}

const onCompositionStart = () => {
  composing.value = true
}

const onCompositionEnd = () => {
  composing.value = false
  compositionJustEnded.value = true
  setTimeout(() => {
    compositionJustEnded.value = false
  }, 100)
}

const isAlwaysOnTop = computed(() => windowStore.workflowWindowAlwaysOnTop)

const workflows = computed(() => workflowStore.workflows)
const currentWorkflow = computed(() => workflowStore.currentWorkflow)
const messages = computed(() => workflowStore.messages)
const isRunning = computed(() => workflowStore.isRunning)
const isAwaitingApproval = computed(() => currentWorkflow.value?.status === 'awaiting_approval')
const currentWorkflowId = computed(() => workflowStore.currentWorkflowId)

// Enhanced messages with pre-calculated display info
const enhancedMessages = computed(() => {
  if (!workflowStore.messages || workflowStore.messages.length === 0) return [];

  const rawMsgs = workflowStore.messages;
  const toolStates = new Map(); // tool_call_id -> { isFinal: bool, isRejected: bool, hasError: bool }
  const toolHasWaitingMsg = new Set(); // tool_call_id that has an 'Awaiting' message

  // --- PASS 1: Single scan to collect all states (O(N)) ---
  const processedMsgs = rawMsgs.map(m => {
    let meta = m.metadata;
    if (typeof meta === 'string') {
      try { meta = JSON.parse(meta); } catch (e) { meta = {}; }
    }

    if (m.role === 'tool' && meta?.tool_call_id) {
      const id = meta.tool_call_id;
      const summary = (meta.summary || '').toLowerCase();
      const isWaiting = summary.includes('awaiting') || summary.includes('待审批');

      if (isWaiting) {
        toolHasWaitingMsg.add(id);
      } else {
        const isRejected = summary.includes('rejected') || m.message.includes('rejected') || m.message.includes('拒绝');
        const isError = m.isError || m.is_error || meta.is_error || false;
        toolStates.set(id, { isFinal: true, isRejected, hasError: isError });
      }
    }
    return { ...m, metadata: meta }; // Cache parsed meta for Pass 2
  });

  // --- PASS 2: Filter and Transform (O(N)) ---
  return processedMsgs.filter(m => {
    // Hide redundancy
    if (m.role === 'tool' && m.metadata?.tool_call_id) {
      const id = m.metadata.tool_call_id;
      const state = toolStates.get(id);

      if (state?.isFinal && !state.hasError) {
        const summary = (m.metadata.summary || '').toLowerCase();
        const isWaiting = summary.includes('awaiting') || summary.includes('待审批');

        // If result is success, hide the result message and keep the waiting one
        if (!isWaiting && toolHasWaitingMsg.has(id)) return false;
        // If we are looking at the waiting message but it's already resolved, we KEEP it (for info)
      }
    }
    return !(m.role === 'user' && m.stepType === 'observe');
  }).map((message, idx) => {
    const toolDisplay = getToolDisplayInfo(message);
    const displayId = message.id || `msg_${message.role}_${message.stepIndex}_${idx}`;

    let isRejected = false;
    let isApproved = false;
    if (message.role === 'tool' && message.metadata?.tool_call_id) {
      const state = toolStates.get(message.metadata.tool_call_id);
      if (state?.isFinal) {
        if (state.isRejected) isRejected = true;
        else isApproved = true;
      }
    }

    // Pre-calculate pending tool calls
    let pendingToolCalls = [];
    const toolCalls = message.metadata?.tool_calls || [];
    if (Array.isArray(toolCalls) && toolCalls.length > 0) {
      pendingToolCalls = toolCalls
        .map(call => {
          const name = call.function?.name || call.name || '';
          const rawArgs = call.function?.arguments || call.arguments || {};
          let args = rawArgs;
          if (typeof rawArgs === 'string') {
            try { args = JSON.parse(rawArgs); } catch (e) { args = {}; }
          }
          const { icon, toolType, action, target } = formatToolTitle(name, args);
          return { id: call.id, icon, toolType, action, target };
        })
        .filter(call => !toolStates.has(call.id) || !toolStates.get(call.id).isFinal);
    }

    return {
      ...message,
      displayId,
      toolDisplay,
      pendingToolCalls,
      isRejected,
      isApproved
    };
  }).filter(m => {
    // Standard visibility logic
    if (m.role === 'tool') {
      const name = m.metadata?.tool_call?.name || m.metadata?.tool_call?.function?.name || '';
      if (name === 'answer_user' || name === 'finish_task') return false;
      return true;
    }
    if (m.role === 'assistant') {
      const hasTextContent = (m.message && m.message.trim()) ||
        (m.reasoning && m.reasoning.trim());
      if (hasTextContent) return true;
      if (m.pendingToolCalls && m.pendingToolCalls.length > 0) return true;
      return false;
    }
    return true;
  });
});

// Get todo list from the store
const todoList = computed(() => workflowStore.todoList)

const filteredWorkflows = computed(() => {
  if (!searchQuery.value) return workflows.value
  return workflows.value.filter(wf =>
    (wf.title || wf.userQuery).toLowerCase().includes(searchQuery.value.toLowerCase())
  )
})

const canSendMessage = computed(
  () => inputMessage.value.trim() !== '' && selectedAgent.value
)

// Watch for state changes to handle UI side effects
watch(() => currentWorkflow.value?.status, (newStatus) => {
  // If state is no longer Paused, we should hide any open approval dialog
  if (newStatus !== 'paused' && approvalVisible.value) {
    approvalVisible.value = false
  }
})

const canSwitchWorkflow = computed(() => {
  // Can't switch if a workflow is currently running
  return !isRunning.value
})

// Watch for workflow changes to update UI
watch(currentWorkflow, newWorkflow => {
  if (newWorkflow) {
    // Scroll to bottom when new workflow is selected
    nextTick(() => {
      scrollToBottom()
    })
  }
})

// Watch for messages to scroll to bottom
watch(
  messages,
  () => {
    nextTick(() => {
      scrollToBottom()
    })
  },
  { deep: true }
)

// Helper to remove <SYSTEM_REMINDER>...</SYSTEM_REMINDER> tags from content
const removeSystemReminder = (content) => {
  if (!content) return ''
  // Handle multiline content and multiple tags
  return content.replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, '').trim()
}

const getDiffMarkdown = (content) => {
  try {
    let data = content
    if (typeof content === 'string') {
      try {
        data = JSON.parse(content)
      } catch (e) {
        return content
      }
    }

    const oldStr = data.old_string !== undefined ? data.old_string : ''
    const newStr = data.new_string !== undefined ? data.new_string : (data.content || '')
    const filePath = data.file_path || data.path || 'file'

    // If it's just raw content without diff semantics, return as code block
    if (data.old_string === undefined && data.new_string === undefined && !data.content) {
      return typeof content === 'string' ? content : JSON.stringify(content, null, 2)
    }

    // Generate standard unidiff-like format for better highlighting
    let diffContent = `File: **${filePath}**\n\n\`\`\`diff\n`
    diffContent += `--- ${filePath}\n`
    diffContent += `+++ ${filePath}\n`

    const UI_LINE_LIMIT = 500 // Limit lines shown in UI for performance
    
    if (data.old_string !== undefined) {
      // For edits, show old and new
      const oldLines = oldStr.split('\n')
      const newLines = newStr.split('\n')
      
      // If either side is too long, truncate for the UI
      const displayOldLines = oldLines.slice(0, UI_LINE_LIMIT)
      const displayNewLines = newLines.slice(0, UI_LINE_LIMIT)
      
      displayOldLines.forEach(line => diffContent += `- ${line}\n`)
      if (oldLines.length > UI_LINE_LIMIT) diffContent += `- ... (${oldLines.length - UI_LINE_LIMIT} lines truncated)\n`
      
      displayNewLines.forEach(line => diffContent += `+ ${line}\n`)
      if (newLines.length > UI_LINE_LIMIT) diffContent += `+ ... (${newLines.length - UI_LINE_LIMIT} lines truncated)\n`
    } else {
      // For new files or overwrites: "- " (empty line) then "+ content"
      diffContent += `- \n` 
      const newLines = newStr.split('\n')
      const displayLines = newLines.slice(0, UI_LINE_LIMIT)
      
      displayLines.forEach(line => diffContent += `+ ${line}\n`)
      if (newLines.length > UI_LINE_LIMIT) {
        diffContent += `+ ... (${newLines.length - UI_LINE_LIMIT} lines truncated)\n`
      }
    }

    diffContent += '```'
    return diffContent
  } catch (e) {
    return typeof content === 'string' ? content : JSON.stringify(content)
  }
}

const parseChoiceContent = (content) => {
  try {
    return JSON.parse(content)
  } catch (e) {
    return { question: content, options: [] }
  }
}

const sendUserChoice = async (option) => {
  if (isRunning.value) return

  // Directly send message using existing logic
  inputMessage.value = option
  onSendMessage()
}

// Helper to parse message content
const getParsedMessage = (message) => {
  let content = message.message || ''
  content = removeSystemReminder(content)
  let toolCalls = []
  const isError = message.isError || message.is_error || false

  try {
    const trimmed = content.trim()
    if (trimmed.startsWith('{')) {
      const parsed = JSON.parse(trimmed)
      let parsedContent = parsed.content || ''
      let parsedToolCalls = parsed.tool_calls || parsed.toolCall || (parsed.tool ? [parsed.tool] : [])

      // Filter out internal tools
      parsedToolCalls = parsedToolCalls.filter(call => {
        const name = call?.function?.name || call?.name
        return name !== 'finish_task' && name !== 'answer_user'
      })

      // If assistant Think step, hide tool calls
      if (message.role === 'assistant' && message.stepType === 'Think') {
        parsedToolCalls = []
      }

      return {
        content: parsedContent,
        toolCalls: parsedToolCalls,
        isError
      }
    }
  } catch (e) {
    // Not JSON
  }

  return {
    content,
    toolCalls: [],
    isError
  }
}

const scrollToBottom = (force = false) => {
  if (messagesRef.value) {
    const el = messagesRef.value
    // Increase threshold to 300px to handle tall tool call blocks
    // If 'force' is true, we scroll regardless of current position
    const isAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 300

    if (force || isAtBottom) {
      nextTick(() => {
        el.scrollTop = el.scrollHeight
      })
    }
  }
}

onMounted(async () => {
  unlistenFocusInput.value = await listen('cs://workflow-focus-input', event => {
    if (event.payload && event.payload.windowLabel === settingStore.windowLabel) {
      if (inputRef.value) {
        inputRef.value.focus()
      }
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
      if (currentWorkflow.value?.status === 'awaiting_approval' && lastMsg?.role === 'tool') {
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
  nextTick(() => scrollToBottom(true))
})

onBeforeUnmount(() => {
  if (unlistenWorkflowEvents.value) {
    unlistenWorkflowEvents.value()
  }
  unlistenFocusInput.value()
  window.removeEventListener('keydown', onGlobalKeyDown)
  window.removeEventListener('resize', updateMaxWidth)
})

const onToggleSidebar = () => {
  sidebarCollapsed.value = !sidebarCollapsed.value
  windowStore.setWorkflowSidebarShow(!sidebarCollapsed.value)
}

const setupWorkflowEvents = async sessionId => {
  if (unlistenWorkflowEvents.value) {
    unlistenWorkflowEvents.value()
    unlistenWorkflowEvents.value = null
  }

  const eventName = `workflow://event/${sessionId}`
  unlistenWorkflowEvents.value = await listen(eventName, event => {
    const payload = event.payload

    if (payload.type === 'state') {
      workflowStore.updateWorkflowStatus(sessionId, payload.state)

      // If we move out of Thinking/Executing, reset the parser
      // Use a small timeout to allow final rendering of streaming buffers
      if (payload.state !== 'thinking' && payload.state !== 'executing') {
        setTimeout(() => {
          chattingParser.reset()
          chatState.value.content = ''
          chatState.value.reasoning = ''
          chatState.value.blocks = []
        }, 500)
      }
    } else if (payload.type === 'chunk') {
      // Direct text chunk from LLM or StreamParser
      chatState.value.content += payload.content
      chatState.value.blocks = chattingParser.process(payload.content)

      scrollToBottom()
    } else if (payload.type === 'reasoning_chunk') {
      // Thinking chunk
      chatState.value.reasoning += payload.content
      scrollToBottom()
    } else if (payload.type === 'message') {
      // ReAct engine sends incremental messages or chunks
      workflowStore.addMessage({
        sessionId: sessionId,
        role: payload.role,
        message: payload.content,
        reasoning: payload.reasoning,
        stepType: payload.step_type,
        stepIndex: payload.step_index,
        isError: payload.is_error,
        errorType: payload.error_type,
        metadata: payload.metadata
      })

      // Message finalized, clear chatting buffer (including reasoning)
      chattingParser.reset()
      chatState.value.content = ''
      chatState.value.reasoning = ''
      chatState.value.blocks = []

      // Force scroll for new full messages
      scrollToBottom(true)
    } else if (payload.type === 'confirm') {
      approvalRequestId.value = payload.id
      approvalAction.value = payload.action
      approvalDetails.value = payload.details
      approvalVisible.value = true
    } else if (payload.type === 'retry_status') {
      // Handle 429 retry status
      chatState.value.retryInfo = {
        attempt: payload.attempt,
        total: payload.total_attempts,
        nextRetryIn: payload.next_retry_in_seconds
      }

      // Auto-decrement timer
      clearRetryTimer()
      retryCountdownTimer = setInterval(() => {
        if (chatState.value.retryInfo && chatState.value.retryInfo.nextRetryIn > 0) {
          chatState.value.retryInfo.nextRetryIn--
        } else {
          clearRetryTimer()
        }
      }, 1000)
    } else if (payload.type === 'sync_todo') {
      workflowStore.setTodoList(payload.todo_list)
    } else if (payload.type === 'compression_status') {
      // Handle context compression status
      isCompressing.value = payload.is_compressing
      compressionMessage.value = payload.message
      if (payload.is_compressing) {
        scrollToBottom(true)
      }
    } else if (payload.type === 'notification') {
      workflowStore.setNotification(payload.message, payload.category)
    }
  })
}

const selectWorkflow = async id => {
  if (!canSwitchWorkflow.value) {
    console.warn('Cannot switch workflow while another is running')
    return
  }

  // Select the workflow in store
  await workflowStore.selectWorkflow(id)

  if (workflowStore.currentWorkflow) {
    const agent = agentStore.agents.find(a => a.id === workflowStore.currentWorkflow.agentId)
    if (agent) {
      selectedAgent.value = agent
      // Setup event listeners for the existing session
      await setupWorkflowEvents(id)
    }

    // Initialize settings from workflow's agentConfig or fallback to agent defaults
    const config = workflowStore.currentWorkflow.agentConfig || {}
    
    // finalAuditMode
    if (config.final_audit !== undefined && config.final_audit !== null) {
      finalAuditMode.value = config.final_audit ? 'on' : 'off'
    } else if (selectedAgent.value?.finalAudit) {
      finalAuditMode.value = 'on'
    } else {
      finalAuditMode.value = 'off'
    }

    // approvalLevel
    if (config.approval_level) {
      approvalLevel.value = config.approval_level
    } else if (selectedAgent.value?.approvalLevel) {
      approvalLevel.value = selectedAgent.value.approvalLevel
    } else {
      approvalLevel.value = 'default'
    }
  }
}

const startNewWorkflow = async (prompt) => {
  if (!selectedAgent.value) {
    console.error('No agent selected')
    return
  }

  if (!prompt || !prompt.trim()) return

  try {
    console.log('Initiating workflow creation...')
    // Get allowed paths: use pendingPaths if any, otherwise fall back to agent's paths
    let workflowAllowedPaths = []
    if (pendingPaths.value.length > 0) {
      workflowAllowedPaths = [...pendingPaths.value]
    } else if (selectedAgent.value.allowedPaths) {
      try {
        workflowAllowedPaths = typeof selectedAgent.value.allowedPaths === 'string'
          ? JSON.parse(selectedAgent.value.allowedPaths)
          : selectedAgent.value.allowedPaths
      } catch (e) {
        console.error('Failed to parse agent allowedPaths:', e)
      }
    }

    // 1. Create workflow in DB first to get a session_id
    const res = await invokeWrapper('create_workflow', {
      request: {
        userQuery: prompt,
        agentId: selectedAgent.value.id,
        allowedPaths: workflowAllowedPaths,
        finalAudit: finalAuditMode.value === 'on'
      }
    })

    const newWorkflowId = typeof res === 'string' ? res : (res.id || res)
    console.log('Workflow session created:', newWorkflowId)

    // Clear pending paths after workflow is created
    pendingPaths.value = []

    // 2. Sync UI state
    await workflowStore.loadWorkflows()
    await workflowStore.selectWorkflow(newWorkflowId)
    await setupWorkflowEvents(newWorkflowId)

    // 4. Trigger engine
    console.log('Calling workflow_start backend command...')
    await invokeWrapper('workflow_start', {
      sessionId: newWorkflowId,
      agentId: selectedAgent.value.id,
      initialPrompt: prompt,
      planningMode: planningMode.value
    })
    console.log('Workflow engine started successfully')
    nextTick(() => scrollToBottom())
  } catch (error) {
    console.error('Failed to start workflow:', error)
    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
  }
}

const onApproveAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: true,
      id: approvalRequestId.value,
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to approve action:', error)
    // If session is lost, force close dialog
    if (String(error).includes('No sender') || String(error).includes('No active session') || String(error).includes('Session interrupted')) {
        showMessage(t('workflow.sessionLost') || 'Session disconnected. Please refresh the page to restore the workflow.', 'warning')
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
    } else {
        showMessage(String(error), 'error')
    }
  } finally {
    approvalLoading.value = false
  }
}

const onApproveAllAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: true,
      approve_all: true,
      id: approvalRequestId.value,
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to approve all actions:', error)
    if (String(error).includes('No sender') || String(error).includes('No active session') || String(error).includes('Session interrupted')) {
        showMessage(t('workflow.sessionLost') || 'Session disconnected. Please refresh the page to restore the workflow.', 'warning')
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
    } else {
        showMessage(String(error), 'error')
    }
  } finally {
    approvalLoading.value = false
  }
}

const onRejectAction = async () => {
  approvalLoading.value = true
  try {
    const signal = JSON.stringify({
      type: 'approval',
      approved: false,
      id: approvalRequestId.value,
    })
    await invokeWrapper('workflow_signal', {
      sessionId: currentWorkflowId.value,
      signal
    })
    approvalVisible.value = false
  } catch (error) {
    console.error('Failed to reject action:', error)
    if (String(error).includes('No sender') || String(error).includes('No active session') || String(error).includes('Session interrupted')) {
        showMessage(t('workflow.sessionLost') || 'Session disconnected. Please refresh the page to restore the workflow.', 'warning')
        approvalVisible.value = false
        // Reset running state since the session is lost
        workflowStore.setRunning(false)
    } else {
        showMessage(String(error), 'error')
    }
  } finally {
    approvalLoading.value = false
  }
}

const handleBuiltinCommand = async (command) => {
  const cmd = command.trim().toLowerCase()

  if (cmd === '/settings') {
    await invokeWrapper('open_setting_window', { settingType: 'general' })
    return true
  }
  if (cmd === '/mcp') {
    await invokeWrapper('open_setting_window', { settingType: 'mcp' })
    return true
  }
  if (cmd === '/proxy') {
    await invokeWrapper('open_setting_window', { settingType: 'proxy' })
    return true
  }
  if (cmd === '/agent') {
    await invokeWrapper('open_setting_window', { settingType: 'agent' })
    return true
  }
  if (cmd === '/about') {
    await invokeWrapper('open_setting_window', { settingType: 'about' })
    return true
  }
  if (cmd === '/models') {
    openModelSelector()
    return true
  }
  return false
}

const onSendMessage = async () => {
  if (!canSendMessage.value) return

  const message = inputMessage.value

  // Handle Builtin UI Commands (Exact match after trim)
  if (message.trim().startsWith('/')) {
    if (await handleBuiltinCommand(message)) {
      inputMessage.value = ''
      return
    }
  }

  inputMessage.value = ''
  console.log('Sending message to workflow:', message)

  // CRITICAL: Reset the stream parser and UI buffer BEFORE sending the new request.
  // This ensures no residual data from the previous turn pollutes the next response.
  chattingParser.reset()
  chatState.value.content = ''
  chatState.value.reasoning = ''
  chatState.value.blocks = []

  if (!currentWorkflowId.value) {
    // Start brand new workflow
    await startNewWorkflow(message)
  } else {
    // 2. Decide: Signal or Re-start?
    const isPaused = currentWorkflow.value?.status === 'paused'
    if (isRunning.value || isPaused) {
      // Just send signal to the running loop
      try {
        const signal = JSON.stringify({
          type: 'user_input',
          content: message
        })

        // Optimistic update to clear the "AI is waiting" hint immediately
        if (isPaused) {
          workflowStore.updateWorkflowStatus(currentWorkflowId.value, 'thinking')
        }

        const res = await invokeWrapper('workflow_signal', {
          sessionId: currentWorkflowId.value,
          signal: signal
        })
        console.log('Signal sent successfully:', res)
      } catch (error) {
        console.error('Failed to send signal:', error)
      }
    } else {
      // Engine is stopped (Completed, Error, or Awaiting Approval).
      // DO NOT add message manually here, workflow_start will handle it and broadcast via events.
      try {
        // If we were awaiting approval, continue in planning mode if we send a message (rejecting the plan)
        const isCurrentlyAwaiting = currentWorkflow.value?.status === 'awaiting_approval'

        await invokeWrapper('workflow_start', {
          sessionId: currentWorkflowId.value,
          agentId: selectedAgent.value.id,
          initialPrompt: message,
          planningMode: isCurrentlyAwaiting || planningMode.value
        })
      } catch (error) {
        console.error('Failed to resume workflow:', error)
        showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
      }
    }
  }
}

const onInputKeyDown = event => {
  if (composing.value || compositionJustEnded.value) return

  // Handle Slash Command Suggestions
  if (showSkillSuggestions.value) {
    if (event.key === 'Enter' || event.key === 'Tab') {
      event.preventDefault()
      if (filteredSystemSkills.value.length > 0) {
        onSkillSelect(filteredSystemSkills.value[selectedSkillIndex.value])
      } else {
        showSkillSuggestions.value = false
      }
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      selectedSkillIndex.value = (selectedSkillIndex.value - 1 + filteredSystemSkills.value.length) % filteredSystemSkills.value.length
      return
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      selectedSkillIndex.value = (selectedSkillIndex.value + 1) % filteredSystemSkills.value.length
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      showSkillSuggestions.value = false
      return
    }
  }

  // Handle File At-mention Suggestions
  if (showFileSuggestions.value) {
    if (event.key === 'Enter' || event.key === 'Tab') {
      event.preventDefault()
      if (fileSuggestions.value.length > 0) {
        onFileSelect(fileSuggestions.value[selectedFileIndex.value])
      } else {
        showFileSuggestions.value = false
      }
      return
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault()
      selectedFileIndex.value = (selectedFileIndex.value - 1 + fileSuggestions.value.length) % fileSuggestions.value.length
      return
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      selectedFileIndex.value = (selectedFileIndex.value + 1) % fileSuggestions.value.length
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      showFileSuggestions.value = false
      return
    }
  }

  if (event.key === 'Enter') {
    const shouldSend =
      settingStore.settings.sendMessageKey === 'Enter' ? !event.shiftKey : event.shiftKey
    if (shouldSend) {
      event.preventDefault()
      onSendMessage()
    }
  }
}

const onContinue = async () => {
  if (!currentWorkflowId.value || isRunning.value) return

  try {
    // If it's paused, we might need to send a signal,
    // but usually 'workflow_start' with no prompt works to resume the loop if it's not active.
    await invokeWrapper('workflow_start', {
      sessionId: currentWorkflowId.value,
      agentId: selectedAgent.value.id
    })
  } catch (error) {
    console.error('Failed to continue workflow:', error)
    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
  }
}

const onApprovePlan = async () => {
  if (!currentWorkflowId.value) return

  // Find the last assistant message that contains 'submit_plan' tool call
  const assistantMsgs = messages.value.filter(m => m.role === 'assistant')
  const lastAssistantMsg = assistantMsgs[assistantMsgs.length - 1]

  if (!lastAssistantMsg) return

  // Extract plan from tool call arguments if available, otherwise use message content
  let planContent = lastAssistantMsg.message
  try {
    const metadata = typeof lastAssistantMsg.metadata === 'string'
      ? JSON.parse(lastAssistantMsg.metadata)
      : lastAssistantMsg.metadata

    if (metadata && (metadata.tool_calls || metadata.tool)) {
      const toolCalls = metadata.tool_calls || (metadata.tool ? [metadata.tool] : [])
      const submitPlanCall = toolCalls.find(c =>
        (c.name === 'submit_plan') ||
        (c.function && c.function.name === 'submit_plan')
      )
      if (submitPlanCall) {
        const args = typeof submitPlanCall.arguments === 'string'
          ? JSON.parse(submitPlanCall.arguments)
          : (submitPlanCall.arguments || submitPlanCall.function?.arguments || submitPlanCall.input)
        if (args && args.plan) {
          planContent = args.plan
        }
      }
    }
  } catch (e) {
    console.warn('Failed to extract plan from metadata, using raw message content instead:', e)
  }

  try {
    await invokeWrapper('workflow_approve_plan', {
      sessionId: currentWorkflowId.value,
      agentId: selectedAgent.value.id,
      plan: planContent
    })
    console.log('Plan approved and execution started')
  } catch (error) {
    console.error('Failed to approve plan:', error)
    showMessage(t('workflow.startFailed', { error: String(error) }), 'error')
  }
}

const onStop = async () => {
  if (currentWorkflowId.value) {
    // Optimistic update: Immediately set running to false to toggle the UI button.
    // The backend might take a moment to gracefully cancel, but the user expects immediate feedback.
    workflowStore.setRunning(false)
    try {
      await invokeWrapper('workflow_stop', {
        sessionId: currentWorkflowId.value
      })
    } catch (error) {
      console.error('Failed to stop workflow:', error)
    }
  }
}

const onPin = () => {
  windowStore.toggleWorkflowWindowAlwaysOnTop()
}

const onEditWorkflow = id => {
  editWorkflowId.value = id
  editWorkflowTitle.value = workflows.value.find(wf => wf.id === id)?.title || ''
  editWorkflowDialogVisible.value = true
}

const onSaveEditWorkflow = async () => {
  if (!editWorkflowId.value) return

  try {
    await invokeWrapper('update_workflow_title', {
      sessionId: editWorkflowId.value,
      title: editWorkflowTitle.value
    })

    // Reload workflows to get updated data
    await workflowStore.loadWorkflows()

    editWorkflowDialogVisible.value = false
    editWorkflowTitle.value = ''
    editWorkflowId.value = null
  } catch (error) {
    console.error('Failed to update workflow:', error)
  }
}

const onDeleteWorkflow = id => {
  ElMessageBox.confirm(t('workflow.confirmDeleteWorkflow'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel')
  }).then(async () => {
    try {
      await invokeWrapper('delete_workflow', { sessionId: id })

      // If deleting the current workflow, clear it
      if (id === currentWorkflowId.value) {
        workflowStore.clearCurrentWorkflow()
      }

      // Reload workflows
      await workflowStore.loadWorkflows()

      // Load the last workflow if available
      if (workflows.value.length > 0) {
        await selectWorkflow(workflows.value[0].id)
      }
    } catch (error) {
      console.error('Failed to delete workflow:', error)
    }
  })
}

const createNewWorkflow = () => {
  // 1. Capture current environment before clearing
  const currentPathsToPreserve = [...currentPaths.value]
  const currentAgentToPreserve = selectedAgent.value

  // 2. Clear only the session-specific state in the store
  workflowStore.clearCurrentWorkflow()

  // 3. Restore environment into local state for the next workflow
  pendingPaths.value = currentPathsToPreserve
  selectedAgent.value = currentAgentToPreserve

  // 4. Reset only the user input
  inputMessage.value = ''

  nextTick(() => {
    if (inputRef.value) {
      inputRef.value.focus()
    }
  })
}

const onGlobalKeyDown = event => {
  // Use OS type from backend. `std::env::consts::OS` returns "macos" for macOS.
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

const toggleFinalAuditMode = () => {
  const newValue = finalAuditMode.value === 'on' ? 'off' : 'on'
  finalAuditMode.value = newValue
  // Persist to database
  if (currentWorkflowId.value) {
    workflowStore.updateWorkflowFinalAudit(currentWorkflowId.value, newValue === 'on')
  }
}
</script>

<style lang="scss">
.sidebar-tabs-container {
  height: 100%;
  display: flex;
  flex-direction: column;

  .sidebar-tabs {
    height: 100%;
    display: flex;
    flex-direction: column;

    :deep(.el-tabs__header) {
      margin: 0;
      padding: 0 15px;
      background: var(--cs-bg-color);
    }

    :deep(.el-tabs__content) {
      flex: 1;
      overflow: hidden;

      .el-tab-pane {
        height: 100%;
        display: flex;
        flex-direction: column;
      }
    }
  }
}

.retry-status-alert {
  margin-top: 12px;
  max-width: 500px;

  .el-alert {
    border-radius: var(--cs-border-radius-lg);
    border: 1px solid var(--el-color-warning-light-5);
    background-color: var(--el-color-warning-light-9);
  }
}

// Context compression status
.compression-status {
  display: flex;
  justify-content: center;
  padding: 16px 0;

  .compression-indicator {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 16px;
    background-color: var(--cs-bg-color-light);
    border-radius: var(--cs-border-radius-xxl);
    border: 1px solid var(--cs-border-color);

    .cs {
      color: var(--el-color-primary);
    }

    .compression-text {
      font-size: 13px;
      color: var(--cs-text-color-secondary);
    }
  }
}

.danger-option {
  color: var(--el-color-danger) !important;
  font-weight: bold;
}

// Approval Level Dropdown Styles
.el-dropdown-menu.approval-level-dropdown .el-dropdown-menu__item {
  display: flex !important;
  flex-direction: row !important;
  align-items: center;
  gap: 8px;

  .dropdown-icon {
    flex-shrink: 0;
    color: var(--cs-text-color-secondary);
  }

  .dropdown-text {
    flex: 1;
    text-align: left;
  }

  .dropdown-check {
    flex-shrink: 0;
    color: var(--el-color-primary);
  }

  &.active {
    .dropdown-icon {
      color: var(--el-color-primary);
    }
  }

  &.danger-option {
    .dropdown-icon {
      color: var(--el-color-danger);
    }

    &.active .dropdown-check {
      color: var(--el-color-danger);
    }
  }
}

.workflow-layout {
  height: 100vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;

  .workflow-main {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: row;

    .sidebar {
      border-right: 1px solid var(--cs-border-color);
      display: flex;
      flex-direction: column;
      height: 100%;
      transition: width 0.3s ease;
      min-width: 0;

      &.dragging {
        transition: none;
      }

      .sidebar-tabs-container {
        height: 100%;
        display: flex;
        flex-direction: column;

        .sidebar-tabs {
          height: 100%;
          display: flex;
          flex-direction: column;

          .el-tabs__header {
            margin: 0;
            padding: 0 var(--cs-space);
            background: var(--cs-bg-color);
          }

          .el-tabs__content {
            flex: 1;
            overflow: hidden;

            .el-tab-pane {
              height: 100%;
              display: flex;
              flex-direction: column;
              padding: 0 var(--cs-space);
            }
          }
        }
      }

      .sidebar-header {
        padding: var(--cs-space) 0;
        flex-shrink: 0;

        .el-input {
          box-sizing: border-box;

          .el-input__wrapper {
            padding: 0;
            background: var(--cs-input-bg-color) !important;
            border-radius: var(--cs-border-radius-xxl);
            font-size: var(--cs-font-size-sm);
          }

          .el-input__prefix {
            display: flex;
            align-items: center;
            padding-left: var(--cs-space-sm);

            .cs {
              font-size: var(--cs-font-size-md);
              color: var(--cs-text-color-secondary);
            }
          }
        }
      }

      .workflow-list {
        flex: 1;
        overflow-y: auto;
        height: calc(100% - 60px);

        .list {
          .item {
            padding: 10px 15px;
            cursor: pointer;
            border-radius: 6px;
            margin-bottom: 2px;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
            transition: background-color 0.2s ease;
            display: flex;
            flex-direction: column;
            gap: 4px;
            position: relative;

            &:hover:not(.disabled) {
              background-color: var(--cs-hover-bg-color);
            }

            &.active {
              background-color: var(--cs-active-bg-color);
              color: var(--el-color-primary);
            }

            &.disabled {
              cursor: not-allowed;
              opacity: 0.6;
            }

            .workflow-title {
              font-weight: 500;
              overflow: hidden;
              text-overflow: ellipsis;
            }

            .workflow-status {
              display: flex;
              align-items: center;
              gap: 6px;
              font-size: var(--cs-font-size-xs);
              color: var(--cs-text-color-secondary);

              .status-indicator {
                width: 6px;
                height: 6px;
                border-radius: 50%;

                &.running {
                  background-color: var(--el-color-warning);
                  animation: pulse 1.5s ease-in-out infinite;
                }

                &.completed {
                  background-color: var(--el-color-success);
                }

                &.paused {
                  background-color: var(--el-color-info);
                }

                &.error {
                  background-color: var(--el-color-danger);
                }
              }
            }

            .icons {
              position: absolute;
              right: 10px;
              top: 50%;
              transform: translateY(-50%);
              display: flex;
              gap: 4px;
              opacity: 0;
              transition: opacity 0.2s ease;

              .icon {
                display: flex;
                align-items: center;
                justify-content: center;
                width: 24px;
                height: 24px;
                border-radius: var(--cs-border-radius-round);
                cursor: pointer;
                color: var(--cs-text-color-secondary);

                &:hover {
                  background-color: var(--cs-bg-color-light);
                  color: var(--cs-text-color-primary);
                }

                .cs {
                  font-size: var(--cs-font-size-sm);
                }
              }
            }

            &:hover .icons {
              opacity: 1;
            }
          }
        }
      }
    }

    .sidebar-resize-handle {
      width: 4px;
      height: 100%;
      background-color: transparent;
      cursor: col-resize;
      position: relative;
      z-index: 100;
      flex-shrink: 0;

      &:hover,
      &.dragging {
        background-color: var(--el-color-primary);
        opacity: 0.5;
      }

      &::before {
        content: '';
        position: absolute;
        left: -4px;
        top: 0;
        width: 12px;
        height: 100%;
      }
    }

    .main-container {
      display: flex;
      flex-direction: column;
      flex: 1;
      overflow: hidden;
      height: 100%;
      position: relative; // For floating panels

      .messages {
        flex: 1;
        overflow-y: auto;
        padding: 15px;
        scroll-behavior: smooth;

        .message {
          display: flex;
          margin-bottom: 20px;
          position: relative;

          .avatar {
            flex-shrink: 0;
            width: 32px;
            height: 32px;
            display: flex;
            align-items: center;
            justify-content: center;
            margin-right: 12px;
            margin-top: 2px;
            border-radius: 50%;
            background-color: var(--cs-bg-color);
            border: 1px solid var(--cs-border-color);

            .user-icon {
              color: var(--el-color-primary);
              font-size: 18px;
            }
          }

          .content-container {
            flex: 1;
            min-width: 0;
            max-width: 90%;
          }

          &.user {
            justify-content: flex-end;

            .avatar {
              display: none; // Removing avatar as requested for a cleaner look if desired, or keep only for user
            }

            .content {
              display: flex;
              flex-direction: row-reverse;
              align-items: flex-start;
              gap: 8px;

              .simple-text {
                padding: var(--cs-space);
                border-radius: var(--cs-border-radius-lg);
                max-width: 100%;
                min-width: 0;
                border: 1px solid var(--cs-border-color);
                margin: 0;
                white-space: pre-wrap;
                word-break: break-all;
                overflow-wrap: anywhere;
                line-height: 1.8;
                background: var(--cs-bg-color-light);
                font-family: inherit;
              }
            }
          }

          &.assistant,
          &.tool {
            position: relative;

            .ai-content {
              .content {
                background: none;
              }

              // CLI Style Tool Calls
              .cli-tool-calls-container {
                margin-bottom: 8px;
              }

              .cli-tool-call {
                font-family: var(--cs-font-family-mono, monospace);
                font-size: 13px;
                line-height: 1.5;
                margin-bottom: 8px;
                display: block; // Force block container
                border-left: 3px solid transparent;
                padding-left: 8px;
                transition: all 0.2s ease;

                // Status-based border colors (override tool type colors)
                &.status-success {
                  border-left-color: var(--el-color-success);
                }

                &.status-error {
                  border-left-color: var(--el-color-danger);

                  .finish-task-display {

                    .finish-text,
                    .cs {
                      color: var(--el-color-danger);

                    }

                    .finish-text {
                      text-decoration: line-through;
                    }
                  }
                }

                &.status-running {
                  border-left-color: var(--el-color-primary);
                  animation: pulse-border 1.5s infinite;
                }

                .expandable {
                  cursor: pointer;
                }

                @keyframes pulse-border {

                  0%,
                  100% {
                    opacity: 1;
                  }

                  50% {
                    opacity: 0.5;
                  }
                }

                // Tool type icon colors
                &.tool-file .tool-type-icon {
                  color: var(--el-color-primary);
                }

                &.tool-network .tool-type-icon {
                  color: var(--el-color-success);
                }

                &.tool-system .tool-type-icon {
                  color: var(--el-color-warning);
                }

                &.tool-todo .tool-type-icon {
                  color: #8b5cf6;
                }

                .tool-line {
                  display: flex;
                  flex-direction: row; // Explicit horizontal
                  align-items: center;
                  white-space: nowrap;
                  width: 100%;
                  gap: var(--cs-space-xs);

                  &.title-wrap {
                    user-select: none;
                    margin-bottom: var(--cs-space-xxs);
                    cursor: pointer;

                    &.tool-rejected {
                      text-decoration: line-through;
                      opacity: 0.6;
                    }

                    .approved-icon {
                      color: var(--el-color-success);
                      margin-left: 4px;
                      flex-shrink: 0;
                    }

                    .tool-type-icon {
                      flex-shrink: 0;
                      width: 14px;
                      height: 14px;
                    }

                    .tool-name {
                      font-weight: 600;
                      color: var(--cs-text-color-primary);
                      flex: 1;
                      min-width: 0;
                      overflow: hidden;
                      text-overflow: ellipsis;
                      white-space: nowrap;
                    }

                    .tool-target {
                      flex: 0 1 auto;
                      max-width: 50%;
                      color: var(--cs-text-color-secondary);
                      font-size: var(--cs-font-size-sm);
                      overflow: hidden;
                      text-overflow: ellipsis;
                      white-space: nowrap;
                    }
                  }

                  &.summary {
                    color: var(--cs-text-color-secondary);
                    padding-left: 4px;
                    align-items: flex-start; // Keep icon at top

                    .corner-icon {
                      font-size: 16px;
                      width: 16px;
                      display: inline-block;
                      text-align: center;
                      flex-shrink: 0;
                      margin-top: -2px;
                      margin-left: var(--cs-space);
                    }

                    .summary-text {
                      font-size: 12px;
                      white-space: nowrap;
                      overflow: hidden;
                      text-overflow: ellipsis;
                      flex: 0 1 auto; // Auto-shrink based on content
                      line-height: 1.4;
                      padding-top: var(--cs-space-xs);
                    }

                    .expand-hint {
                      font-size: 11px;
                      opacity: 0.4;
                      margin-left: 4px; // Closer to text
                      flex: 0 0 auto; // Keep fixed width
                      line-height: 1.4;
                      padding-top: var(--cs-space-xs); // Match summary-text padding
                    }
                  }
                }

                // Summary text color for error state
                &.status-error .summary-text {
                  color: var(--el-color-danger);
                }

                &.pending {
                  opacity: 0.8;

                  .tool-name,
                  .tool-target {
                    color: var(--cs-text-color-placeholder);
                  }
                }

                // finish_task special display
                .finish-task-display {
                  display: flex;
                  align-items: center;
                  gap: var(--cs-space-xs);
                  padding: var(--cs-space-xs) 0;

                  .finish-icon {
                    color: var(--el-color-success);
                  }

                  .finish-text {
                    color: var(--el-color-success);
                    font-weight: 600;
                    font-size: 14px;
                  }
                }

                .tool-detail {
                  margin-top: 8px;
                  margin-left: 20px;
                  padding: 12px;
                  background-color: var(--cs-bg-color-light);
                  border-radius: var(--cs-border-radius-sm);
                  border-left: 2px solid var(--cs-border-color);
                  font-family: var(--cs-font-family-mono, monospace);

                  .choice-container {
                    padding: 12px;
                    background: var(--cs-bg-color-light);
                    border-radius: var(--cs-border-radius-md);
                    margin-top: 8px;

                    .choice-question {
                      font-size: var(--cs-font-size-sm);
                      margin-bottom: 12px;
                      color: var(--cs-text-color-primary);
                      line-height: 1.5;
                    }

                    .choice-options {
                      display: flex;
                      flex-wrap: wrap;
                      gap: 8px;
                    }
                  }

                  .raw-content {
                    margin: 0;
                    white-space: pre-wrap;
                    word-break: break-all;
                    font-size: 12px;
                    color: var(--cs-text-color-regular);
                    background: none;
                    border: none;
                    padding: 0;
                    max-height: 300px;
                    overflow: auto;
                  }
                }
              }

              // Thoughts - collapsible reasoning container
              .reasoning-container {
                margin-bottom: 12px;

                .reasoning-header {
                  display: flex;
                  align-items: center;
                  gap: 8px;
                  cursor: pointer;
                  padding: 4px 0;

                  .reasoning-icon {
                    color: var(--cs-text-color-secondary);
                    font-size: 14px;

                    &.rotating {
                      animation: cs-rotate 2s linear infinite;
                    }
                  }

                  .reasoning-text {
                    color: var(--cs-text-color-secondary);
                    font-style: italic;
                    font-size: 13px;
                    overflow: hidden;
                    text-overflow: ellipsis;
                    white-space: nowrap;
                  }

                  .reasoning-toggle {
                    color: var(--cs-text-color-placeholder);
                    font-size: 12px;

                    &:hover {
                      color: var(--cs-color-primary);
                    }
                  }
                }

                .reasoning-content {
                  margin-top: 8px;
                  padding: 8px 12px;
                  background-color: var(--cs-bg-color);
                  border-radius: var(--cs-border-radius-sm);
                  border-left: 3px solid var(--cs-border-color-light);
                  color: var(--cs-text-color-secondary);
                  font-style: italic;
                  font-size: 13px;
                  line-height: 1.6;
                  white-space: pre-wrap;
                  animation: slideDown 0.2s ease;
                }

                @keyframes slideDown {
                  from {
                    opacity: 0;
                    max-height: 0;
                    margin-top: 0;
                  }

                  to {
                    opacity: 1;
                    max-height: 500px;
                    margin-top: 8px;
                  }
                }
              }

              // Legacy thought-content (keep for compatibility)
              .thought-content {
                margin-bottom: 12px;
                color: var(--cs-text-color-secondary);
                font-style: italic;
                font-size: 13px;
                line-height: 1.6;
                padding: 8px 12px;
                background-color: var(--cs-bg-color);
                border-radius: var(--cs-border-radius-sm);
                border-left: 3px solid var(--cs-border-color-light);
                white-space: pre-wrap;
              }

              // Custom <thought> tag styling
              thought {
                display: block;
                font-style: italic;
                color: var(--cs-text-color-secondary);
                background-color: var(--cs-bg-color-light);
                padding: 8px 12px;
                border-left: 3px solid var(--cs-border-color-light);
                border-radius: var(--cs-border-radius);
                margin: 10px 0;
                font-size: 0.9em;
                line-height: 1.5;
                white-space: pre-wrap;

                &::before {
                  content: "Thought";
                  display: block;
                  font-weight: bold;
                  font-size: 0.8em;
                  text-transform: uppercase;
                  margin-bottom: 4px;
                  opacity: 0.6;
                }
              }
            }
          }

          &.observe {
            opacity: 0.9;
            font-size: 0.95em;
          }
        }
      }

      .todo-floating-panel {
        position: absolute;
        bottom: 100%; // Sit on top of the footer
        left: 20px;
        right: 20px;
        z-index: 100;
        pointer-events: none; // Allow clicks to pass through to messages below
        display: flex;
        justify-content: center;

        // Make the list itself interactive
        :deep(.todo-list) {
          pointer-events: auto;
          background-color: var(--cs-bg-color);
          border: 1px solid var(--cs-border-color);
          border-radius: var(--cs-border-radius-lg);
          box-shadow: var(--el-box-shadow-light);
          padding: 8px 12px;
          max-width: 600px;
          width: 100%;
          opacity: 0.95;
          backdrop-filter: blur(4px);
          transition: opacity 0.3s ease;

          &:hover {
            opacity: 1;
          }
        }
      }

      footer.input-container {
        flex-shrink: 0;
        background-color: transparent;
        padding: 0 var(--cs-space-sm) var(--cs-space-sm);
        height: unset;
        z-index: 1;
        position: relative;

        .input-header {
          display: flex;
          justify-content: flex-start;
          margin-bottom: 8px;
          padding-left: 10px;

          .model-selector-trigger {
            display: inline-flex;
            align-items: center;
            gap: 6px;
            padding: 4px 10px;
            background: var(--cs-bg-color-light);
            border: 1px solid var(--cs-border-color);
            border-radius: var(--cs-border-radius-lg);
            cursor: pointer;
            font-size: 11px;
            color: var(--cs-text-color-secondary);
            transition: all 0.2s;
            backdrop-filter: blur(10px);
            opacity: 0.8;

            &:hover {
              opacity: 1;
              background: var(--cs-bg-color-hover);
              color: var(--cs-text-color-primary);
              border-color: var(--el-color-primary-light-5);
            }

            .model-name {
              font-weight: 500;
            }
          }
        }

        .slash-command-panel {
          position: absolute;
          bottom: calc(100% - 10px);
          left: var(--cs-space-sm);
          right: var(--cs-space-sm);
          background-color: var(--cs-bg-color);
          border: 1px solid var(--cs-border-color);
          border-radius: var(--cs-border-radius-lg);
          box-shadow: var(--el-box-shadow-light);
          max-height: 300px;
          overflow-y: auto;
          z-index: 100;
          padding: 4px;

          &.file-suggestion-panel.compact {
            padding: 2px;

            .command-item {
              flex-direction: row;
              align-items: center;
              padding: 6px 10px;
              gap: 8px;

              .file-icon {
                flex-shrink: 0;
                color: var(--cs-text-color-secondary);
                font-size: 14px;
              }

              .file-path {
                flex: 1;
                font-size: 13px;
                color: var(--cs-text-color-primary);
                overflow: hidden;
                text-overflow: ellipsis;
                white-space: nowrap;
                font-family: var(--cs-font-family-mono, monospace);
              }

              &.active {
                background-color: var(--cs-active-bg-color);

                .file-path,
                .file-icon {
                  color: var(--el-color-primary);
                }
              }
            }
          }

          .command-item {

            padding: 8px 12px;
            cursor: pointer;
            border-radius: var(--cs-border-radius-sm);
            display: flex;
            flex-direction: column;
            gap: 2px;

            &:hover,
            &.active {
              background-color: var(--cs-hover-bg-color);
            }

            .command-name {
              font-weight: 600;
              font-size: 13px;
              color: var(--cs-color-primary);
            }

            .command-desc {
              font-size: 12px;
              color: var(--cs-text-color-secondary);
              white-space: nowrap;
              overflow: hidden;
              text-overflow: ellipsis;
            }
          }
        }

        .additional {
          display: flex;
          gap: 1px;
          margin-bottom: var(--cs-space-xs);

          .additional-item {
            display: flex;
            align-items: center;
            flex: 1;
            max-width: 50%;
            background-color: var(--cs-input-bg-color);
            border-radius: var(--cs-border-radius-xxl);
            padding: var(--cs-space-xs);
            box-sizing: border-box;

            .data {
              flex: 1;
              min-width: 0;

              .skill-item {
                padding: 0;
              }

              .message-text {
                padding-left: var(--cs-space);
                display: block;
                white-space: nowrap;
                overflow: hidden;
                text-overflow: ellipsis;
                color: var(--cs-text-color-secondary);
                font-size: var(--cs-font-size-sm);
                line-height: 1.5;
                position: relative;

                &:before {
                  position: absolute;
                  top: -3px;
                  left: 3px;
                }
              }
            }

            .close-btn {
              display: flex;
              align-items: center;
              justify-content: center;
              width: 24px;
              height: 24px;
              margin-left: var(--cs-space-xs);
              flex-shrink: 0;
              cursor: pointer;
              border-radius: var(--cs-border-radius-round);
              color: var(--cs-text-color-secondary);

              &:hover {
                background-color: var(--cs-bg-color-light);
              }
            }
          }
        }

        .input {
          display: flex;
          flex-direction: column;
          background-color: var(--cs-input-bg-color);
          border-radius: var(--cs-border-radius-lg);
          padding: var(--cs-space-sm) var(--cs-space) var(--cs-space-xs);

          .input-status-hint {
            display: flex;
            flex-direction: column;
            gap: 8px;
            font-size: 11px;
            color: var(--el-color-primary);
            padding: 8px;
            background: var(--el-color-primary-light-9);
            border-radius: var(--cs-border-radius-sm);
            margin-bottom: 8px;

            .hint-header {
              display: flex;
              align-items: center;
              gap: 6px;
            }

            .hint-options {
              display: flex;
              flex-wrap: wrap;
              gap: 6px;
              padding-left: 18px;

              .el-button {
                font-size: 11px;
                height: 24px;
                padding: 0 8px;
                margin: 0;
                background: var(--cs-bg-color);
                border-color: var(--el-color-primary-light-7);

                &:hover {
                  background: var(--el-color-primary);
                  color: white;
                }
              }
            }
          }

          .icons {
            display: flex;
            align-items: center;
            justify-content: center;
            padding: var(--cs-space-xs);
            cursor: pointer;
            gap: var(--cs-space-xs);

            .cs {
              font-size: var(--cs-font-size-xl) !important;
              color: var(--cs-text-color-secondary);

              &.small {
                font-size: var(--cs-font-size-md) !important;
              }

              &.cs-send:not(.disabled) {
                color: var(--cs-color-primary);
              }
            }

            label {
              font-size: var(--cs-font-size-sm);
              display: flex;
              align-items: center;
              justify-content: center;
              cursor: pointer;
              color: var(--cs-text-color-secondary);
              background-color: var(--cs-bg-color);
              border-radius: var(--cs-border-radius-lg);
              padding: var(--cs-space-xs) var(--cs-space-sm);
              border: 1px solid var(--cs-bg-color);

              &:not(.disabled):not(.default):hover,
              &.active {
                color: var(--cs-color-primary);

                .cs {
                  color: var(--cs-color-primary);
                }
              }

              &.active {
                border: 1px solid var(--cs-color-primary);
              }
            }
          }

          .el-textarea {
            flex-grow: 1;

            .el-textarea__inner {
              border: none;
              box-shadow: none;
              background: var(--cs-input-bg-color) !important;
              resize: none !important;
              color: var(--cs-text-color-primary);
              padding-left: var(--cs-space-xxs);
              padding-right: var(--cs-space-xxs);
            }
          }

          .input-footer {
            display: flex;
            flex-direction: row;
            align-items: center;
            justify-content: space-between;

            .footer-left {
              display: flex;
              flex-direction: row;
              justify-content: flex-start;
              align-items: center;
              gap: 8px;

              .final-audit-toggle {
                display: flex;
                align-items: center;
                gap: 4px;
                cursor: pointer;
                transition: all 0.2s;

                &:hover {
                  background: var(--cs-hover-bg-color);
                }

                &.on {
                  color: var(--el-color-success);
                  border: 1px solid var(--el-color-success);
                }

                &.off {
                  opacity: 0.6;
                }

                .cs {
                  font-size: 14px;
                }

                .audit-label {
                  font-size: 9px;
                  font-weight: bold;
                  color: var(--cs-text-color-secondary);
                }
              }

              .warning-mode {
                .cs {
                  color: var(--el-color-danger) !important;
                  animation: pulse 2s infinite;
                }
              }

              .agent-selector-wrap {
                color: var(--cs-color-primary);
                background: var(--cs-bg-color);
                border: 1px solid var(--cs-color-primary);
                border-radius: var(--cs-border-radius-lg);
                padding: var(--cs-space-xs) var(--cs-space-sm);
                font-size: var(--cs-font-size-md);

                &.disabled {
                  border-color: var(--cs-border-color);
                  background: none;
                }
              }

              /* Authorized paths wrap removed - now only in sidebar tab */
            }
          }
        }
      }
    }
  }
}

.pin-btn {
  border-radius: var(--cs-border-radius-xs);
  color: var(--cs-text-color-secondary);

  &:hover .cs {
    color: var(--cs-color-primary) !important;
  }

  .cs {
    font-size: var(--cs-font-size-md) !important;
    transform: rotate(45deg);
    transition: all 0.3s ease-in-out;
  }

  &.active {
    .cs {
      color: var(--cs-color-primary);
      transform: rotate(0deg);
    }
  }
}

@keyframes pulse {
  0% {
    opacity: 1;
  }

  50% {
    opacity: 0.5;
  }

  100% {
    opacity: 1;
  }
}

.rotating {
  animation: cs-rotate 2s linear infinite;
  display: inline-block;
  margin-right: 4px;
}

@keyframes cs-rotate {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
</style>
