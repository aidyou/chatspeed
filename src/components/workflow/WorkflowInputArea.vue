<template>
  <el-footer class="input-container">
    <!-- Slash Command Suggestion Panel -->
    <div v-if="showSkillSuggestions && filteredSystemSkills.length > 0" class="slash-command-panel">
      <div v-for="group in groupedSkillSuggestions" :key="group.key" class="command-group">
        <div class="command-group-header">
          <div class="command-group-title">{{ group.title }}</div>
          <button
            v-if="group.key === 'installed'"
            type="button"
            class="command-group-action"
            :title="$t('workflow.skillsConfigTitle')"
            @mousedown.prevent
            @click.stop="$emit('open-skills-selector')">
            <cs name="setting" size="13px" />
          </button>
        </div>
        <div
          v-for="skill in group.items"
          :key="`${group.key}-${skill.name}`"
          class="command-item"
          :class="{ active: skill.originalIndex === selectedSkillIndex }"
          @click="onSkillSelect(skill)">
          <div class="command-name">/{{ skill.name }}</div>
          <div class="command-desc">{{ skill.description }}</div>
        </div>
      </div>
    </div>

    <!-- File At-mention Suggestion Panel -->
    <div
      v-if="showFileSuggestions && fileSuggestions.length > 0"
      class="slash-command-panel file-suggestion-panel compact">
      <div
        v-for="(file, idx) in fileSuggestions"
        :key="file.path"
        class="command-item"
        :class="{ active: idx === selectedFileIndex }"
        @click="onFileSelect(file)">
        <cs :name="file.is_directory ? 'folder' : 'file'" size="14px" class="file-icon" />
        <span class="file-path">{{ file.relative_path }}</span>
        <!-- Show root hint for non-primary directories -->
        <span
          v-if="
            file.root_path &&
            props.currentPaths?.length > 0 &&
            file.root_path !== props.currentPaths[0]
          "
          class="file-root-hint">
          ({{
            file.root_path
              .split(/[/\\]/)
              .filter(p => p !== '')
              .pop()
          }})
        </span>
      </div>
    </div>
    <StatusNotifier :chat-state="chatState" :is-chatting="isChatting" />
    <div class="input">
      <div v-if="attachments.length > 0" class="workflow-attachments">
        <div
          v-for="attachment in attachments"
          :key="attachment.id"
          class="workflow-attachment-item">
          <div
            v-if="attachment.uploading"
            class="workflow-attachment-preview workflow-attachment-preview-loading">
            <span class="workflow-attachment-spinner" />
          </div>
          <img
            v-else
            :src="attachment.url"
            :alt="attachment.name"
            class="workflow-attachment-preview" />
          <span class="workflow-attachment-name">{{ attachment.name }}</span>
          <span v-if="attachment.uploading" class="workflow-attachment-status">
            {{ $t('chat.preparingAttachments') }}
          </span>
          <cs
            name="close"
            class="workflow-attachment-remove"
            @click="$emit('remove-attachment', attachment.id)" />
        </div>
      </div>
      <el-input
        ref="inputRef"
        v-model="inputMessage"
        type="textarea"
        :autosize="{ minRows: 1, maxRows: 10 }"
        :placeholder="$t('chat.inputMessagePlaceholder', { at: '/' })"
        @keydown="onInputKeyDown"
        @compositionstart="onCompositionStart"
        @compositionend="onCompositionEnd"
        @paste="handlePaste" />

      <div class="input-footer">
        <div class="footer-left">
          <div v-if="canEditAgent" class="selector-wrap">
            <AgentSelector
              :model-value="selectedAgent"
              :agent="null"
              :disabled="false"
              @update:model-value="$emit('update-selected-agent', $event)" />
          </div>
          <div class="selector-wrap model-selector-trigger" @click="$emit('open-model-selector')">
            <span class="model-name">{{ activeModelName }}</span>
            <cs name="arrow-down" size="12px" />
          </div>

          <div class="icons">
            <el-dropdown trigger="click" @command="handleQuickActionCommand">
              <label class="icon-btn upperLayer">
                <cs name="add" class="small" />
              </label>
              <template #dropdown>
                <el-dropdown-menu class="workflow-quick-actions-dropdown">
                  <el-dropdown-item v-if="canAttachImages" command="attachment">
                    <cs name="attachment" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-text">{{ $t('chat.addAttachment') }}</span>
                    </span>
                  </el-dropdown-item>
                  <el-dropdown-item
                    v-if="showPlanningModeToggle"
                    command="planning"
                    :disabled="!canTogglePlanningMode"
                    :class="{ active: planningMode }">
                    <cs name="skill-plan" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-main">
                        <span class="dropdown-text">{{ $t('settings.agent.planningMode') }}</span>
                        <cs v-if="planningMode" name="check" size="14px" class="dropdown-check" />
                      </span>
                      <span class="dropdown-note">{{ $t('workflow.planningModeTooltip') }}</span>
                    </span>
                  </el-dropdown-item>
                  <el-dropdown-item
                    command="finalAudit"
                    :disabled="!canToggleFinalAuditMode"
                    :class="{ active: finalAuditMode !== 'off' }">
                    <cs name="check-circle" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-main">
                        <span class="dropdown-text">{{ $t('settings.agent.finalAudit') }}</span>
                        <cs
                          v-if="finalAuditMode !== 'off'"
                          name="check"
                          size="14px"
                          class="dropdown-check" />
                      </span>
                      <span class="dropdown-note">{{ $t('workflow.finalAuditTooltip') }}</span>
                    </span>
                  </el-dropdown-item>
                  <el-dropdown-item command="autoCompress" :class="{ active: autoCompressEnabled }">
                    <cs name="compress" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-main">
                        <span class="dropdown-text">{{ autoCompressMenuLabel }}</span>
                        <cs
                          v-if="autoCompressEnabled"
                          name="check"
                          size="14px"
                          class="dropdown-check" />
                      </span>
                      <span class="dropdown-note">{{ $t('workflow.autoCompressTooltip') }}</span>
                    </span>
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>

            <!-- Approval Level Dropdown -->
            <el-dropdown trigger="click" @command="$emit('update-approval-level', $event)">
              <label
                class="icon-btn upperLayer"
                :class="{ 'warning-mode': approvalLevel === 'full' }">
                <cs
                  :name="
                    approvalLevel === 'default'
                      ? 'setting'
                      : approvalLevel === 'smart'
                        ? 'brain'
                        : 'yolo'
                  "
                  class="small" />
              </label>
              <template #dropdown>
                <el-dropdown-menu class="approval-level-dropdown">
                  <el-dropdown-item
                    command="default"
                    :class="{ active: approvalLevel === 'default' }">
                    <cs name="setting" size="14px" class="dropdown-icon" />
                    <span class="dropdown-text">{{
                      $t('settings.agent.approvalLevelDefault')
                    }}</span>
                    <cs
                      v-if="approvalLevel === 'default'"
                      name="check"
                      size="14px"
                      class="dropdown-check" />
                  </el-dropdown-item>
                  <el-dropdown-item command="smart" :class="{ active: approvalLevel === 'smart' }">
                    <cs name="brain" size="14px" class="dropdown-icon" />
                    <span class="dropdown-text">{{ $t('settings.agent.approvalLevelSmart') }}</span>
                    <cs
                      v-if="approvalLevel === 'smart'"
                      name="check"
                      size="14px"
                      class="dropdown-check" />
                  </el-dropdown-item>
                  <el-dropdown-item
                    command="full"
                    class="danger-option"
                    :class="{ active: approvalLevel === 'full' }">
                    <cs name="yolo" size="14px" class="dropdown-icon" />
                    <span class="dropdown-text">{{ $t('settings.agent.approvalLevelFull') }}</span>
                    <cs
                      v-if="approvalLevel === 'full'"
                      name="check"
                      size="14px"
                      class="dropdown-check" />
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>

            <!-- Auto-Approved Tools & Shell Commands Popover -->
            <el-popover
              v-if="approvalLevel === 'default'"
              placement="top"
              :width="360"
              trigger="click"
              popper-class="auto-approved-popover">
              <template #reference>
                <label
                  class="icon-btn upperLayer auto-approve-badge"
                  :class="{ 'has-items': autoApprovedItemCount > 0 }">
                  <cs name="tool" class="small" />
                  <span v-if="autoApprovedItemCount > 0" class="badge">
                    {{ autoApprovedItemCount }}
                  </span>
                </label>
              </template>

              <div class="auto-approved-panel">
                <!-- Auto-Approved Tools Section -->
                <div class="panel-section">
                  <div class="section-header">
                    <cs name="tool" size="14px" class="section-icon" />
                    <span class="section-title">{{ $t('workflow.autoApprovedTools') }}</span>
                    <span class="section-count">{{ autoApprovedTools.length }}</span>
                  </div>
                  <div
                    v-if="availableApprovalTools.length > 0"
                    class="section-content checkbox-list">
                    <label
                      v-for="tool in availableApprovalTools"
                      :key="tool.id"
                      class="checkbox-item tool-checkbox-item">
                      <el-checkbox
                        :model-value="autoApprovedTools.includes(tool.id)"
                        @change="checked => toggleAutoApprovedTool(tool.id, checked)">
                        <span class="checkbox-label-wrap">
                          <code class="tool-name">{{ tool.id }}</code>
                          <span v-if="tool.name && tool.name !== tool.id" class="tool-desc">
                            {{ tool.name }}
                          </span>
                        </span>
                      </el-checkbox>
                    </label>
                  </div>
                  <div v-else class="section-empty-text">
                    {{ $t('common.noData') || 'No tools available' }}
                  </div>
                </div>

                <div class="section-divider"></div>

                <!-- Allowed Shell Commands Section -->
                <div class="panel-section">
                  <div class="section-header">
                    <cs name="skill-terminal" size="14px" class="section-icon" />
                    <span class="section-title">{{
                      $t('workflow.allowedShellCommands') || 'Allowed Shell Patterns'
                    }}</span>
                    <span class="section-count">{{ allowedShellCommands.length }}</span>
                  </div>
                  <el-space>
                    <div class="section-toolbar">
                      <el-input
                        v-model="shellCommandSearch"
                        size="small"
                        clearable
                        :placeholder="$t('common.search') || 'Search shell command pattern'" />
                    </div>
                    <div class="section-toolbar">
                      <el-input
                        v-model="newShellCommandPattern"
                        size="small"
                        clearable
                        :placeholder="
                          $t('settings.agent.shellPolicyPattern') || 'Enter shell command pattern'
                        "
                        @keydown.enter.prevent="addShellPolicyItem">
                        <template #append>
                          <el-button
                            size="small"
                            :disabled="!canAddShellPolicyItem"
                            @click="addShellPolicyItem">
                            {{ $t('settings.agent.shellPolicyAdd') || 'Add' }}
                          </el-button>
                        </template>
                      </el-input>
                    </div>
                  </el-space>
                  <div v-if="filteredAllowedShellCommands.length > 0" class="section-content">
                    <div
                      v-for="(cmd, idx) in filteredAllowedShellCommands"
                      :key="idx"
                      class="tool-item shell-item">
                      <div class="tool-info">
                        <code class="tool-name shell-pattern">{{ cmd.pattern }}</code>
                        <span v-if="cmd.description" class="tool-desc">{{ cmd.description }}</span>
                      </div>
                      <el-button
                        size="small"
                        type="danger"
                        text
                        class="remove-btn"
                        @click="removeShellPolicyItem(cmd.pattern)">
                        <cs name="trash" size="12px" />
                      </el-button>
                    </div>
                  </div>
                  <div v-else class="section-empty-text">
                    {{
                      shellCommandSearch
                        ? $t('common.noData') || 'No matching shell command patterns'
                        : $t('workflow.noAutoApprovedItems') || 'No auto-approved items'
                    }}
                  </div>
                  <div class="section-footer">
                    <div class="section-footer-hint">
                      <cs name="info" size="12px" />
                      <span>{{
                        $t('workflow.shellPolicyClickRemove') || 'Click × to remove items'
                      }}</span>
                    </div>
                    <el-tooltip
                      placement="top"
                      :content="$t('settings.agent.shellPolicyImportDefault')"
                      :hide-after="0"
                      :enterable="false">
                      <button
                        type="button"
                        class="section-footer-action"
                        :disabled="isImportingShellPolicies || !currentWorkflowId"
                        @click="importDefaultShellPolicies">
                        <cs name="import" size="12px" />
                      </button>
                    </el-tooltip>
                  </div>
                </div>
              </div>
            </el-popover>

            <el-tooltip
              v-if="currentWorkflowId"
              :content="
                canClearContext
                  ? $t('workflow.clearContextFrame')
                  : $t('workflow.clearContextFrameNotStopped')
              "
              :hide-after="0"
              :enterable="false"
              placement="top">
              <label
                class="clear-context-action"
                :class="{ disabled: !canClearContext }"
                @click="canClearContext && $emit('clear-context-frame')">
                <cs name="clear-context" class="small" />
              </label>
            </el-tooltip>

            <el-tooltip
              :content="$t('workflow.newWorkflow')"
              :hide-after="0"
              :enterable="false"
              placement="top">
              <label @click="openCreateWorkflowDialog">
                <cs name="new-chat" class="small" />
              </label>
            </el-tooltip>
          </div>
        </div>
        <div class="icons">
          <el-button
            v-if="canApprovePlan"
            size="small"
            round
            type="success"
            @click="$emit('approve-plan')">
            {{ $t('workflow.approvePlan') }}
          </el-button>
          <el-button
            v-if="canContinue && currentWorkflowId"
            size="small"
            round
            type="primary"
            @click="$emit('continue')">
            {{ $t('workflow.continue') }}
          </el-button>
          <el-button v-else-if="isStopping" size="small" round disabled>
            {{ $t('workflow.stopping') }}
          </el-button>
          <cs name="stop" @click="$emit('stop')" v-if="canStop" />
          <cs name="send" @click="$emit('send-message')" :class="{ disabled: !canSendMessage }" />
        </div>
      </div>
    </div>
    <el-dialog
      v-model="createWorkflowDialogVisible"
      :title="$t('workflow.newWorkflowDialog.title')"
      width="420px"
      append-to-body
      @keydown.capture="handleCreateWorkflowDialogKeydown">
      <div class="new-workflow-options">
        <button
          type="button"
          class="new-workflow-option"
          :class="{ selected: createWorkflowInheritCurrent }"
          :disabled="!currentWorkflow"
          @click="createWorkflowInheritCurrent = true">
          <span class="new-workflow-option-title">
            {{ $t('workflow.newWorkflowDialog.inheritTitle') }}
          </span>
          <span class="new-workflow-option-description">
            {{ $t('workflow.newWorkflowDialog.inheritDescription') }}
          </span>
          <cs v-if="createWorkflowInheritCurrent" name="check" size="16px" class="new-workflow-option-check" />
        </button>
        <button
          type="button"
          class="new-workflow-option"
          :class="{ selected: !createWorkflowInheritCurrent }"
          @click="createWorkflowInheritCurrent = false">
          <span class="new-workflow-option-title">
            {{ $t('workflow.newWorkflowDialog.defaultTitle') }}
          </span>
          <span class="new-workflow-option-description">
            {{ $t('workflow.newWorkflowDialog.defaultDescription') }}
          </span>
          <cs v-if="!createWorkflowInheritCurrent" name="check" size="16px" class="new-workflow-option-check" />
        </button>
      </div>
      <template #footer>
        <el-button type="primary" @click="createWorkflowFromSelectedMode">{{ $t('common.confirm') }}</el-button>
      </template>
    </el-dialog>
  </el-footer>
</template>

<script setup>
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import AgentSelector from './AgentSelector.vue'
import StatusNotifier from './StatusNotifier.vue'

const props = defineProps({
  isRunning: {
    type: Boolean,
    default: false
  },
  isChatting: {
    type: Boolean,
    default: false
  },
  hasLiveSession: {
    type: Boolean,
    default: false
  },
  chatState: {
    type: Object,
    default: () => ({
      content: '',
      reasoning: '',
      reasoningStatus: 'idle'
    })
  },
  waitReason: {
    type: String,
    default: null
  },
  currentWorkflow: {
    type: Object,
    default: null
  },
  currentWorkflowId: {
    type: String,
    default: null
  },
  selectedAgent: {
    type: Object,
    default: null
  },
  canEditAgent: {
    type: Boolean,
    default: true
  },
  activeModelName: {
    type: String,
    default: 'Select Model'
  },
  showPlanningModeToggle: {
    type: Boolean,
    default: true
  },
  planningMode: {
    type: Boolean,
    default: false
  },
  canTogglePlanningMode: {
    type: Boolean,
    default: true
  },
  approvalLevel: {
    type: String,
    default: 'default'
  },
  finalAuditMode: {
    type: String,
    default: 'off'
  },
  canToggleFinalAuditMode: {
    type: Boolean,
    default: true
  },
  autoCompressEnabled: {
    type: Boolean,
    default: true
  },
  agents: {
    type: Array,
    default: () => []
  },
  attachments: {
    type: Array,
    default: () => []
  },
  canAttachImages: {
    type: Boolean,
    default: false
  },
  isPreparingImageSend: {
    type: Boolean,
    default: false
  },
  showSkillSuggestions: {
    type: Boolean,
    default: false
  },
  showFileSuggestions: {
    type: Boolean,
    default: false
  },
  filteredSystemSkills: {
    type: Array,
    default: () => []
  },
  groupedSkillSuggestions: {
    type: Array,
    default: () => []
  },
  fileSuggestions: {
    type: Array,
    default: () => []
  },
  selectedSkillIndex: {
    type: Number,
    default: 0
  },
  selectedFileIndex: {
    type: Number,
    default: 0
  },
  onInputKeyDown: {
    type: Function,
    required: true
  },
  onCompositionStart: {
    type: Function,
    required: true
  },
  onCompositionEnd: {
    type: Function,
    required: true
  },
  onPasteInput: {
    type: Function,
    default: null
  },
  onSkillSelect: {
    type: Function,
    required: true
  },
  onFileSelect: {
    type: Function,
    required: true
  }
})

const emit = defineEmits([
  'send-message',
  'continue',
  'stop',
  'approve-plan',
  'toggle-planning-mode',
  'toggle-final-audit-mode',
  'toggle-auto-compress',
  'update-approval-level',
  'update-selected-agent',
  'clear-context-frame',
  'create-new-workflow',
  'open-model-selector',
  'open-skills-selector',
  'open-image-dialog',
  'remove-attachment'
])

import { useWorkflowStore } from '@/stores/workflow'
import { useAgentStore } from '@/stores/agent'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const agentStore = useAgentStore()
const defaultShellPolicies = ref([])
const isImportingShellPolicies = ref(false)
const newShellCommandPattern = ref('')
const shellCommandSearch = ref('')

const autoApprovedTools = computed(() =>
  [...workflowStore.autoApprovedTools].sort((a, b) => a.localeCompare(b))
)
const allowedShellCommands = computed(() =>
  [...workflowStore.allowedShellCommands].sort((a, b) => a.pattern.localeCompare(b.pattern))
)
const filteredAllowedShellCommands = computed(() => {
  const keyword = shellCommandSearch.value.trim().toLowerCase()
  if (!keyword) return allowedShellCommands.value

  return allowedShellCommands.value.filter(cmd => {
    const pattern = String(cmd.pattern || '').toLowerCase()
    const description = String(cmd.description || '').toLowerCase()
    return pattern.includes(keyword) || description.includes(keyword)
  })
})
const availableApprovalTools = computed(() => {
  const allowedToolIds = Array.isArray(props.selectedAgent?.availableTools)
    ? props.selectedAgent.availableTools
    : Array.isArray(props.currentWorkflow?.agentConfig?.availableTools)
      ? props.currentWorkflow.agentConfig.availableTools
      : []

  const allowedSet = new Set(
    allowedToolIds.filter(toolId => toolId && toolId !== 'bash' && toolId !== 'mcp_tool_load')
  )

  return agentStore.availableTools
    .filter(tool => allowedSet.has(tool.id))
    .map(tool => ({
      id: tool.id,
      name: tool.name || tool.id
    }))
    .sort((a, b) => a.id.localeCompare(b.id, 'zh-Hans'))
})
const autoApprovedItemCount = computed(
  () => autoApprovedTools.value.length + allowedShellCommands.value.length
)
const canAddShellPolicyItem = computed(() =>
  Boolean(props.currentWorkflowId && newShellCommandPattern.value.trim())
)

// Phase 3: Use semantic computed fields from store for UI control
const canStop = computed(() => workflowStore.canStop)
const canContinue = computed(() => workflowStore.canContinue)
const canApprovePlan = computed(() => workflowStore.canApprovePlan)
const isStopping = computed(() => workflowStore.isStopping)
const canClearContext = computed(() => workflowStore.canClearContext)

const buildNextAgentConfig = overrides => {
  const currentAgentConfig = props.currentWorkflow?.agentConfig || {}
  return {
    ...currentAgentConfig,
    ...overrides
  }
}

const persistAgentConfig = async overrides => {
  if (!props.currentWorkflowId) return false

  const nextAgentConfig = buildNextAgentConfig(overrides)

  await invokeWrapper('update_workflow_agent_config', {
    sessionId: props.currentWorkflowId,
    agentConfig: JSON.stringify(nextAgentConfig)
  })

  if (props.currentWorkflow) {
    props.currentWorkflow.agentConfig = nextAgentConfig
    if (Object.prototype.hasOwnProperty.call(overrides, 'shellPolicy')) {
      props.currentWorkflow.shellPolicy = nextAgentConfig.shellPolicy || []
    }
  }

  return nextAgentConfig
}

const toggleAutoApprovedTool = async (toolName, checked) => {
  if (!props.currentWorkflowId) return

  const currentAutoApprove = Array.isArray(props.currentWorkflow?.agentConfig?.autoApprove)
    ? props.currentWorkflow.agentConfig.autoApprove
    : [...workflowStore.autoApprovedTools]

  const nextAutoApprove = checked
    ? [...new Set([...currentAutoApprove, toolName])]
    : currentAutoApprove.filter(tool => tool !== toolName)

  try {
    await persistAgentConfig({ autoApprove: nextAutoApprove })
    workflowStore.setAutoApprovedTools(nextAutoApprove)
  } catch (error) {
    console.error('Failed to toggle auto-approved tool:', error)
  }
}

const removeAutoApprovedTool = async toolName => {
  await toggleAutoApprovedTool(toolName, false)
}

const removeShellPolicyItem = async pattern => {
  try {
    await invokeWrapper('remove_shell_policy_item', {
      sessionId: props.currentWorkflowId,
      pattern
    })
    workflowStore.removeShellPolicyItem(pattern)
  } catch (error) {
    console.error('Failed to remove shell policy item:', error)
  }
}

const ensureDefaultShellPoliciesLoaded = async () => {
  if (defaultShellPolicies.value.length > 0) return

  const result = await invokeWrapper('get_default_shell_policy')
  defaultShellPolicies.value = Array.isArray(result) ? result : []
}

const addShellPolicyItem = async () => {
  const pattern = newShellCommandPattern.value.trim()
  if (!props.currentWorkflowId || !pattern) return

  const currentPolicy = Array.isArray(props.currentWorkflow?.agentConfig?.shellPolicy)
    ? props.currentWorkflow.agentConfig.shellPolicy
    : Array.isArray(props.currentWorkflow?.shellPolicy)
      ? props.currentWorkflow.shellPolicy
      : []

  const exists = currentPolicy.some(
    rule => rule.pattern === pattern && (rule.decision || 'review') === 'allow'
  )
  if (exists) {
    showMessage(t('common.noData') || 'Pattern already exists', 'info')
    return
  }

  const nextPolicy = [...currentPolicy, { pattern, decision: 'allow' }]

  try {
    await persistAgentConfig({ shellPolicy: nextPolicy })
    workflowStore.setShellPolicy(nextPolicy)
    newShellCommandPattern.value = ''
  } catch (error) {
    console.error('Failed to add shell policy item:', error)
  }
}

const importDefaultShellPolicies = async () => {
  if (!props.currentWorkflowId || isImportingShellPolicies.value) return

  isImportingShellPolicies.value = true
  try {
    await ensureDefaultShellPoliciesLoaded()

    const currentPolicy = Array.isArray(props.currentWorkflow?.agentConfig?.shellPolicy)
      ? props.currentWorkflow.agentConfig.shellPolicy
      : Array.isArray(props.currentWorkflow?.shellPolicy)
        ? props.currentWorkflow.shellPolicy
        : []

    const mergedPolicy = [...currentPolicy]
    defaultShellPolicies.value.forEach(defaultRule => {
      const exists = mergedPolicy.some(
        rule => rule.pattern === defaultRule.pattern && rule.decision === defaultRule.decision
      )
      if (!exists) {
        mergedPolicy.push({ ...defaultRule })
      }
    })

    const mergedCount = mergedPolicy.length - currentPolicy.length
    if (mergedCount <= 0) {
      showMessage(t('common.noData') || 'No new rules to import', 'info')
      return
    }

    await persistAgentConfig({ shellPolicy: mergedPolicy })
    workflowStore.setShellPolicy(mergedPolicy)
    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    console.error('Failed to import default shell policy:', error)
    showMessage('Failed to import default shell policy', 'error')
  } finally {
    isImportingShellPolicies.value = false
  }
}

const inputRef = ref(null)
const createWorkflowDialogVisible = ref(false)
const createWorkflowInheritCurrent = ref(true)

const inputMessage = defineModel('inputMessage', { type: String, default: '' })

const autoCompressMenuLabel = computed(() => t('workflow.autoCompressShort'))

const openCreateWorkflowDialog = () => {
  createWorkflowInheritCurrent.value = Boolean(props.currentWorkflow)
  createWorkflowDialogVisible.value = true
}

const createWorkflowFromSelectedMode = () => {
  createWorkflowDialogVisible.value = false
  emit('create-new-workflow', { inheritCurrent: createWorkflowInheritCurrent.value })
}

const handleCreateWorkflowDialogKeydown = event => {
  if (!createWorkflowDialogVisible.value) return

  if (event.key === 'Enter') {
    event.preventDefault()
    createWorkflowFromSelectedMode()
    return
  }

  if (event.key === 'ArrowUp' || event.key === 'ArrowDown' || event.key === 'Tab') {
    event.preventDefault()
    if (props.currentWorkflow) {
      createWorkflowInheritCurrent.value = !createWorkflowInheritCurrent.value
    }
  }
}

const handleQuickActionCommand = command => {
  if (command === 'attachment') {
    emit('open-image-dialog')
    return
  }

  if (command === 'planning') {
    if (props.showPlanningModeToggle && props.canTogglePlanningMode) {
      emit('toggle-planning-mode')
    }
    return
  }

  if (command === 'finalAudit') {
    if (props.canToggleFinalAuditMode) {
      emit('toggle-final-audit-mode')
    }
    return
  }

  if (command === 'autoCompress') {
    emit('toggle-auto-compress')
  }
}

const canSendMessage = computed(
  () =>
    (inputMessage.value.trim() !== '' || props.attachments.length > 0) &&
    props.selectedAgent &&
    !props.attachments.some(attachment => attachment.uploading) &&
    !props.isPreparingImageSend &&
    !isStopping.value
)

const canEditAgent = computed(() => props.canEditAgent)

const handlePaste = event => {
  if (!props.canAttachImages || typeof props.onPasteInput !== 'function') {
    return
  }
  props.onPasteInput(event)
}

defineExpose({
  inputRef,
  focus: () => inputRef.value?.focus(),
  openCreateWorkflowDialog
})
</script>

<style scoped lang="scss">
.new-workflow-options {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm, 8px);
}

.new-workflow-option {
  position: relative;
  display: block;
  width: 100%;
  padding: var(--cs-space-md, 16px);
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-base, 8px);
  background: var(--cs-bg-color);
  color: var(--cs-text-color);
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.2s ease,
    background-color 0.2s ease;
}

.new-workflow-option:hover:not(:disabled) {
  border-color: var(--el-color-primary-light-5);
  background: var(--el-color-primary-light-9);
}

.new-workflow-option.selected {
  border-color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
}

.new-workflow-option:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.new-workflow-option-title,
.new-workflow-option-description {
  display: block;
  padding-right: 24px;
}

.new-workflow-option-title {
  font-size: var(--cs-font-size-md, 16px);
  font-weight: 600;
  font-style: normal;
  line-height: 1.5;
}

.new-workflow-option-description {
  margin-top: var(--cs-space-xs, 4px);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-sm);
  line-height: 1.5;
}

.new-workflow-option-check {
  position: absolute;
  top: var(--cs-space-md, 16px);
  right: var(--cs-space-md, 16px);
  color: var(--el-color-primary);
}

.workflow-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 10px;
}

.workflow-attachment-item {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  max-width: 220px;
  padding: 6px 10px;
  border: 1px solid var(--cs-border-color);
  border-radius: 10px;
  background: var(--cs-bg-elevated, var(--cs-bg-color));
}

.workflow-attachment-preview {
  width: 36px;
  height: 36px;
  border-radius: 6px;
  object-fit: cover;
  flex-shrink: 0;
}

.workflow-attachment-preview-loading {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--cs-fill-color-light, rgba(0, 0, 0, 0.06));
}

.workflow-attachment-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid var(--cs-border-color);
  border-top-color: var(--el-color-primary);
  border-radius: 50%;
  animation: workflow-attachment-spin 0.8s linear infinite;
}

.workflow-attachment-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--cs-font-size-sm);
}

.workflow-attachment-status {
  flex-shrink: 0;
  font-size: 12px;
  color: var(--cs-text-secondary);
}

.workflow-attachment-remove {
  cursor: pointer;
  flex-shrink: 0;
  color: var(--cs-text-secondary);
}

.checkbox-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.checkbox-item {
  display: block;
}

.tool-checkbox-item :deep(.el-checkbox) {
  display: flex;
  align-items: flex-start;
  width: 100%;
  margin-right: 0;
}

.tool-checkbox-item :deep(.el-checkbox__label) {
  min-width: 0;
  flex: 1;
}

.checkbox-label-wrap {
  display: inline-flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.section-toolbar {
  margin-bottom: 10px;
}

.section-empty-text {
  font-size: 12px;
  color: var(--cs-text-secondary);
}

.section-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.section-footer-hint {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.section-footer-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--cs-text-secondary);
  cursor: pointer;
  transition:
    background-color 0.2s ease,
    color 0.2s ease;
}

.section-footer-action:hover:not(:disabled) {
  background: var(--cs-fill-color-light, rgba(0, 0, 0, 0.06));
  color: var(--cs-text-color);
}

.section-footer-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.clear-context-action.disabled {
  opacity: 0.45;
  cursor: not-allowed;
  pointer-events: none;
}

.workflow-quick-actions-dropdown :deep(.el-dropdown-menu__item) {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: var(--cs-space-xs);
}

.dropdown-icon {
  flex-shrink: 0;
  margin-top: 2px;
}

.dropdown-content {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
}

.dropdown-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.dropdown-text {
  min-width: 0;
  line-height: 1.4;
  color: var(--cs-text-color);
}

.dropdown-note {
  margin-top: 2px;
  font-size: 11px;
  line-height: 1.3;
  color: var(--cs-text-secondary);
  white-space: normal;
}

.dropdown-check {
  margin-left: auto;
  flex-shrink: 0;
}

@keyframes workflow-attachment-spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
</style>
