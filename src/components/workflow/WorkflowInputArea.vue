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
      </div>
    </div>

    <div class="input">
      <div v-if="currentWorkflow?.status === 'paused'" class="input-status-hint">
        <div class="hint-header">
          <cs name="talk" size="12px" />
          <span>{{ activeAskUser ? activeAskUser.question : 'AI is waiting for your response...' }}</span>
        </div>
        <div v-if="activeAskUser" class="hint-options">
          <el-button v-for="opt in activeAskUser.options" :key="opt" size="small" plain round @click="inputMessage = opt">
            {{ opt }}
          </el-button>
        </div>
      </div>
      <StatusNotifier />
      <div class="input-header">
        <div class="model-selector-trigger" @click="$emit('open-model-selector')">
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
            <AgentSelector
              :model-value="selectedAgent"
              :agent="currentWorkflow?.agentId
                ? agents.find(a => a.id === currentWorkflow.agentId)
                : null
              "
              :disabled="!!currentWorkflowId"
            />
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
            v-if="!isRunning && !isAwaitingApproval && currentWorkflowId && currentWorkflow?.status !== 'completed' && currentWorkflow?.status !== 'error'"
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
