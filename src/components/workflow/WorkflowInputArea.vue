<template>
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
        <!-- Show root hint for non-primary directories -->
        <span v-if="file.root_path && props.currentPaths?.length > 0 && file.root_path !== props.currentPaths[0]"
          class="file-root-hint">
          ({{file.root_path.split(/[/\\]/).filter(p => p !== '').pop()}})
        </span>
      </div>
    </div>
    <StatusNotifier />
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

      <el-input ref="inputRef" v-model="inputMessage" type="textarea" :autosize="{ minRows: 1, maxRows: 10 }"
        :placeholder="$t('chat.inputMessagePlaceholder', { at: '/' })" @keydown="onInputKeyDown"
        @compositionstart="onCompositionStart" @compositionend="onCompositionEnd" />

      <div class="input-footer">
        <div class="footer-left">
          <div class="selector-wrap" :class="{ disabled: currentWorkflowId }">
            <AgentSelector :model-value="selectedAgent" :agent="currentWorkflow?.agentId
              ? agents.find(a => a.id === currentWorkflow.agentId)
              : null
              " :disabled="!!currentWorkflowId" />
          </div>
          <div class="selector-wrap model-selector-trigger" @click="$emit('open-model-selector')">
            <span class="model-name">{{ activeModelName }} ({{ planningMode ? 'plan' : 'act' }})</span>
            <cs name="arrow-down" size="12px" />
          </div>

          <div class="icons">
            <el-tooltip :content="$t('workflow.planningModeTooltip')" placement="top">
              <label class="icon-btn upperLayer" :class="{ active: planningMode }"
                @click="$emit('toggle-planning-mode')">
                <cs name="skill-plan" class="small" />
              </label>
            </el-tooltip>

            <!-- Final Audit Toggle -->
            <el-tooltip :content="$t('workflow.finalAuditTooltip')" placement="top">
              <label class="final-audit-toggle icon-btn upperLayer" :class="finalAuditMode"
                @click="$emit('toggle-final-audit-mode')">
                <cs name="check-circle" class="small" />
                <span class="audit-label" v-if="finalAuditMode !== 'off'">{{ finalAuditMode.toUpperCase()
                }}</span>
              </label>
            </el-tooltip>

            <!-- Approval Level Dropdown -->
            <el-dropdown trigger="click" @command="$emit('update-approval-level', $event)">
              <label class="icon-btn upperLayer" :class="{ 'warning-mode': approvalLevel === 'full' }">
                <cs :name="approvalLevel === 'default' ? 'setting' : (approvalLevel === 'smart' ? 'brain' : 'warning')"
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
                  <el-dropdown-item command="full" class="danger-option" :class="{ active: approvalLevel === 'full' }">
                    <cs name="warning" size="14px" class="dropdown-icon" />
                    <span class="dropdown-text">{{ $t('settings.agent.approvalLevelFull') }}</span>
                    <cs v-if="approvalLevel === 'full'" name="check" size="14px" class="dropdown-check" />
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>

            <!-- Auto-Approved Tools & Shell Commands Popover -->
            <el-popover
              v-if="approvalLevel === 'default' && (autoApprovedTools.length > 0 || allowedShellCommands.length > 0)"
              placement="top"
              :width="360"
              trigger="click"
              popper-class="auto-approved-popover"
            >
              <template #reference>
                <label class="icon-btn upperLayer auto-approve-badge has-items">
                  <cs name="tool" class="small" />
                  <span v-if="autoApprovedTools.length > 0 || allowedShellCommands.length > 0" class="badge">
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
                        @click="removeAutoApprovedTool(tool)"
                      >
                        <cs name="close" size="12px" />
                      </el-button>
                    </div>
                  </div>
                </div>

                <!-- Divider -->
                <div v-if="autoApprovedTools.length > 0 && allowedShellCommands.length > 0" class="section-divider"></div>

                <!-- Allowed Shell Commands Section -->
                <div v-if="allowedShellCommands.length > 0" class="panel-section">
                  <div class="section-header">
                    <cs name="skill-terminal" size="14px" class="section-icon" />
                    <span class="section-title">{{ $t('workflow.allowedShellCommands') || 'Allowed Shell Patterns' }}</span>
                    <span class="section-count">{{ allowedShellCommands.length }}</span>
                  </div>
                  <div class="section-content">
                    <div v-for="(cmd, idx) in allowedShellCommands" :key="idx" class="tool-item shell-item">
                      <div class="tool-info">
                        <code class="tool-name shell-pattern">{{ cmd.pattern }}</code>
                        <span v-if="cmd.description" class="tool-desc">{{ cmd.description }}</span>
                      </div>
                      <el-button
                        size="small"
                        type="danger"
                        text
                        class="remove-btn"
                        @click="removeShellPolicyItem(cmd.pattern)"
                      >
                        <cs name="close" size="12px" />
                      </el-button>
                    </div>
                  </div>
                  <div class="section-footer">
                    <cs name="info" size="12px" />
                    <span>{{ $t('workflow.shellPolicyClickRemove') || 'Click × to remove items' }}</span>
                  </div>
                </div>

                <!-- Empty State -->
                <div v-if="autoApprovedTools.length === 0 && allowedShellCommands.length === 0" class="empty-state">
                  <cs name="tool" size="32px" class="empty-icon" />
                  <span class="empty-text">{{ $t('workflow.noAutoApprovedItems') || 'No auto-approved items' }}</span>
                </div>
              </div>
            </el-popover>

            <el-tooltip :content="$t('workflow.newWorkflow')" :hide-after="0" :enterable="false" placement="top">
              <label @click="$emit('create-new-workflow')" :class="{ disabled: isRunning }">
                <cs name="new-chat" class="small" :class="{ disabled: isRunning }" />
              </label>
            </el-tooltip>
          </div>
        </div>
        <div class="icons">
          <el-button v-if="isAwaitingApproval" size="small" round type="success" @click="$emit('approve-plan')">
            {{ $t('workflow.approvePlan') }}
          </el-button>
          <el-button
            v-if="!isRunning && !isAwaitingApproval && currentWorkflowId && currentWorkflow?.status !== 'pending' && currentWorkflow?.status !== 'completed' && currentWorkflow?.status !== 'error'"
            size="small" round type="primary" @click="$emit('continue')">
            {{ $t('workflow.continue') }}
          </el-button>
          <cs name="stop" @click="$emit('stop')" v-if="isRunning" />
          <cs name="send" @click="$emit('send-message')" :class="{ disabled: !canSendMessage }" />
        </div>
      </div>
    </div>
  </el-footer>
</template>

<script setup>
import { ref, computed } from 'vue'
import AgentSelector from './AgentSelector.vue'
import StatusNotifier from './StatusNotifier.vue'

const props = defineProps({
  isRunning: {
    type: Boolean,
    default: false
  },
  isAwaitingApproval: {
    type: Boolean,
    default: false
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
  activeModelName: {
    type: String,
    default: 'Select Model'
  },
  planningMode: {
    type: Boolean,
    default: false
  },
  approvalLevel: {
    type: String,
    default: 'default'
  },
  finalAuditMode: {
    type: String,
    default: 'off'
  },
  agents: {
    type: Array,
    default: () => []
  },
  activeAskUser: {
    type: Object,
    default: null
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
  'update-approval-level',
  'create-new-workflow',
  'open-model-selector'
])

import { useWorkflowStore } from '@/stores/workflow'
import { invokeWrapper } from '@/libs/tauri'

const workflowStore = useWorkflowStore()

const autoApprovedTools = computed(() => workflowStore.autoApprovedTools)
const allowedShellCommands = computed(() => workflowStore.allowedShellCommands)

const removeAutoApprovedTool = async (toolName) => {
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

const removeShellPolicyItem = async (pattern) => {
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

const inputRef = ref(null)

const inputMessage = defineModel('inputMessage', { type: String, default: '' })

const canSendMessage = computed(
  () => inputMessage.value.trim() !== '' && props.selectedAgent
)

defineExpose({
  inputRef,
  focus: () => inputRef.value?.focus()
})
</script>
