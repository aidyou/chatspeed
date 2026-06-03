<template>
  <el-footer class="input-container">
    <!-- Slash Command Suggestion Panel -->
    <div v-if="showSkillSuggestions && filteredSystemSkills.length > 0" class="slash-command-panel">
      <div
        v-for="group in groupedSkillSuggestions"
        :key="group.key"
        class="command-group">
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
    <StatusNotifier />
    <div class="input">
      <div v-if="attachments.length > 0" class="workflow-attachments">
        <div v-for="attachment in attachments" :key="attachment.id" class="workflow-attachment-item">
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
          <div class="selector-wrap" :class="{ disabled: !canEditAgent }">
            <AgentSelector
              :model-value="selectedAgent"
              :agent="
                currentWorkflow?.agentId && !canEditAgent
                  ? agents.find(a => a.id === currentWorkflow.agentId)
                  : null
              "
              :disabled="!canEditAgent"
              @update:model-value="$emit('update-selected-agent', $event)" />
          </div>
          <div class="selector-wrap model-selector-trigger" @click="$emit('open-model-selector')">
            <span class="model-name">{{ activeModelName }}</span>
            <cs name="arrow-down" size="12px" />
          </div>

          <div class="icons">
            <el-tooltip
              v-if="canAttachImages"
              placement="top"
              :content="$t('chat.addAttachment')"
              :hide-after="0"
              :enterable="false">
              <label class="icon-btn upperLayer" @click="$emit('open-image-dialog')">
                <cs name="attachment" class="small" />
              </label>
            </el-tooltip>

            <el-tooltip
              v-if="showPlanningModeToggle"
              placement="top"
              :content="$t('workflow.planningModeTooltip')"
              :hide-after="0"
              :enterable="false">
              <label
                class="icon-btn upperLayer"
                :class="{ active: planningMode, disabled: !canTogglePlanningMode }"
                @click="canTogglePlanningMode && $emit('toggle-planning-mode')">
                <cs name="skill-plan" class="small" />
              </label>
            </el-tooltip>

            <!-- Final Audit Toggle -->
            <el-tooltip
              placement="top"
              :content="$t('workflow.finalAuditTooltip')"
              :hide-after="0"
              :enterable="false">
              <label
                class="final-audit-toggle icon-btn upperLayer"
                :class="{ [finalAuditMode]: true, disabled: !canToggleFinalAuditMode }"
                @click="canToggleFinalAuditMode && $emit('toggle-final-audit-mode')">
                <cs name="check-circle" class="small" />
                <span class="audit-label" v-if="finalAuditMode !== 'off'">{{
                  finalAuditMode.toUpperCase()
                }}</span>
              </label>
            </el-tooltip>

            <el-tooltip
              placement="top"
              :content="$t('workflow.autoCompressTooltip')"
              :hide-after="0"
              :enterable="false">
              <label
                class="icon-btn upperLayer"
                :class="{ active: autoCompressEnabled }"
                @click="$emit('toggle-auto-compress')">
                <cs name="compress" class="small" />
              </label>
            </el-tooltip>

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
                        : 'warning'
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
                    <cs name="warning" size="14px" class="dropdown-icon" />
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
              v-if="
                approvalLevel === 'default' &&
                (autoApprovedTools.length > 0 || allowedShellCommands.length > 0)
              "
              placement="top"
              :width="360"
              trigger="click"
              popper-class="auto-approved-popover">
              <template #reference>
                <label class="icon-btn upperLayer auto-approve-badge has-items">
                  <cs name="tool" class="small" />
                  <span
                    v-if="autoApprovedTools.length > 0 || allowedShellCommands.length > 0"
                    class="badge">
                    {{ autoApprovedTools.length + allowedShellCommands.length }}
                  </span>
                </label>
              </template>

              <div class="auto-approved-panel">
                <!-- Auto-Approved Tools Section -->
                <div v-if="autoApprovedTools.length > 0" class="panel-section">
                  <div class="section-header">
                    <cs name="tool" size="14px" class="section-icon" />
                    <span class="section-title">{{ $t('workflow.autoApprovedTools') }}</span>
                    <span class="section-count">{{ autoApprovedTools.length }}</span>
                  </div>
                  <div class="section-content">
                    <div v-for="tool in autoApprovedTools" :key="tool" class="tool-item">
                      <div class="tool-info">
                        <code class="tool-name">{{ tool }}</code>
                      </div>
                      <el-button
                        size="small"
                        type="danger"
                        text
                        class="remove-btn"
                        @click="removeAutoApprovedTool(tool)">
                        <cs name="trash" size="12px" />
                      </el-button>
                    </div>
                  </div>
                </div>

                <!-- Divider -->
                <div
                  v-if="autoApprovedTools.length > 0 && allowedShellCommands.length > 0"
                  class="section-divider"></div>

                <!-- Allowed Shell Commands Section -->
                <div v-if="allowedShellCommands.length > 0" class="panel-section">
                  <div class="section-header">
                    <cs name="skill-terminal" size="14px" class="section-icon" />
                    <span class="section-title">{{
                      $t('workflow.allowedShellCommands') || 'Allowed Shell Patterns'
                    }}</span>
                    <span class="section-count">{{ allowedShellCommands.length }}</span>
                  </div>
                  <div class="section-content">
                    <div
                      v-for="(cmd, idx) in allowedShellCommands"
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

                <!-- Empty State -->
                <div
                  v-if="autoApprovedTools.length === 0 && allowedShellCommands.length === 0"
                  class="empty-state">
                  <cs name="tool" size="32px" class="empty-icon" />
                  <span class="empty-text">{{
                    $t('workflow.noAutoApprovedItems') || 'No auto-approved items'
                  }}</span>
                </div>
              </div>
            </el-popover>

            <el-tooltip
              :content="$t('workflow.newWorkflow')"
              :hide-after="0"
              :enterable="false"
              placement="top">
              <label @click="$emit('create-new-workflow')">
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
          <el-button
            v-else-if="isStopping"
            size="small"
            round
            disabled>
            {{ $t('workflow.stopping') }}
          </el-button>
          <cs name="stop" @click="$emit('stop')" v-if="canStop" />
          <cs name="send" @click="$emit('send-message')" :class="{ disabled: !canSendMessage }" />
        </div>
      </div>
    </div>
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
  hasLiveSession: {
    type: Boolean,
    default: false
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

defineEmits([
  'send-message',
  'continue',
  'stop',
  'approve-plan',
  'toggle-planning-mode',
  'toggle-final-audit-mode',
  'toggle-auto-compress',
  'update-approval-level',
  'update-selected-agent',
  'create-new-workflow',
  'open-model-selector',
  'open-skills-selector',
  'open-image-dialog',
  'remove-attachment'
])

import { useWorkflowStore } from '@/stores/workflow'
import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'

const { t } = useI18n()
const workflowStore = useWorkflowStore()
const defaultShellPolicies = ref([])
const isImportingShellPolicies = ref(false)

const autoApprovedTools = computed(() =>
  [...workflowStore.autoApprovedTools].sort((a, b) => a.localeCompare(b))
)
const allowedShellCommands = computed(() =>
  [...workflowStore.allowedShellCommands].sort((a, b) => a.pattern.localeCompare(b.pattern))
)

// Phase 3: Use semantic computed fields from store for UI control
const canStop = computed(() => workflowStore.canStop)
const canContinue = computed(() => workflowStore.canContinue)
const canApprovePlan = computed(() => workflowStore.canApprovePlan)
const isStopping = computed(() => workflowStore.isStopping)

const removeAutoApprovedTool = async toolName => {
  try {
    await invokeWrapper('remove_auto_approved_tool', {
      sessionId: props.currentWorkflowId,
      toolName
    })
    workflowStore.removeAutoApprovedTool(toolName)
  } catch (error) {
    console.error('Failed to remove auto-approved tool:', error)
  }
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

    const nextAgentConfig = {
      ...(props.currentWorkflow?.agentConfig || {}),
      shellPolicy: mergedPolicy
    }

    await invokeWrapper('update_workflow_agent_config', {
      sessionId: props.currentWorkflowId,
      agentConfig: JSON.stringify(nextAgentConfig)
    })

    if (props.currentWorkflow) {
      props.currentWorkflow.agentConfig = nextAgentConfig
      props.currentWorkflow.shellPolicy = mergedPolicy
    }
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

const inputMessage = defineModel('inputMessage', { type: String, default: '' })

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
  focus: () => inputRef.value?.focus()
})
</script>

<style scoped lang="scss">
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

.final-audit-toggle.disabled {
  cursor: not-allowed;
  opacity: 0.5;
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
