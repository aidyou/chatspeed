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
    <div class="input" :class="{ expanded: isInputExpanded }">
      <button
        type="button"
        class="input-expand-toggle"
        @click="isInputExpanded = !isInputExpanded">
        <cs :name="isInputExpanded ? 'fullscreen' : 'fullscreen-off'" />
      </button>
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
          <el-popover
            v-model:visible="modelSelectorOpen"
            placement="top-start"
            :width="320"
            trigger="click"
            popper-class="workflow-model-selector-popover">
            <template #reference>
              <button
                type="button"
                class="selector-wrap model-selector-trigger"
                :class="{ open: modelSelectorOpen }">
                <span class="model-name">{{ activeModelName }}</span>
                <cs name="arrow-down" size="12px" />
              </button>
            </template>
            <div ref="modelSelectorRef" class="workflow-model-selector">
              <template v-for="group in modelOptions" :key="group.key">
                <div
                  v-for="option in group.models"
                  :key="option.key"
                  :class="{ active: option.selected }"
                  class="model-option"
                  role="button"
                  tabindex="0"
                  @click="selectModel(option)"
                  @keydown.enter.prevent="selectModel(option)"
                  @keydown.space.prevent="selectModel(option)">
                  <span class="model-option-main">
                    <span class="model-option-name">{{ option.name }}</span>
                    <cs v-if="option.selected" name="check" size="14px" />
                  </span>
                  <span v-if="option.supportsThinking && option.selected" class="model-thinking-budget" @click.stop>
                    <span class="model-thinking-label">{{ $t('settings.model.thinkingLevel') }}</span>
                    <button
                      v-for="level in thinkingLevelOptions"
                      :key="level.value"
                      type="button"
                      class="thinking-level-option"
                      :class="{ active: option.thinkingLevel === level.value }"
                      @click.stop="updateThinkingLevel(option, level.value)">
                      {{ $t(level.label) }}
                    </button>
                  </span>
                </div>
              </template>
            </div>
          </el-popover>

          <div class="icons">
            <el-dropdown
              ref="quickActionsDropdownRef"
              trigger="click"
              :hide-on-click="false"
              @command="handleQuickActionCommand">
              <label
                class="icon-btn upperLayer quick-actions-badge"
                :class="{ 'has-active-options': activeRuntimeOptionCount > 0 }">
                <cs name="add" class="small" />
                <span v-if="activeRuntimeOptionCount > 0" class="badge">
                  {{ activeRuntimeOptionCount }}
                </span>
              </label>
              <template #dropdown>
                <el-dropdown-menu class="workflow-quick-actions-dropdown">
                  <el-dropdown-item v-if="canAttachImages" command="attachment">
                    <cs name="attachment" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-text">{{ $t('chat.addAttachment') }}</span>
                    </span>
                  </el-dropdown-item>
                  <el-dropdown-item command="manualCompress">
                    <cs name="compress" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-text">{{ $t('workflow.manualCompressShort') }}</span>
                    </span>
                  </el-dropdown-item>
                  <div class="quick-actions-divider" />
                  <div class="quick-actions-section-title">
                    {{ $t('workflow.quickActionsConfiguration') }}
                  </div>
                  <el-dropdown-item command="modelConfig">
                    <cs name="model" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-text">{{ $t('settings.model.modelConfig') }}</span>
                    </span>
                  </el-dropdown-item>
                  <el-dropdown-item command="skillsConfig">
                    <cs name="skill" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-text">{{ $t('workflow.skillsConfigTitle') }}</span>
                    </span>
                  </el-dropdown-item>
                  <div class="quick-actions-divider" />
                  <div class="quick-actions-section-title">
                    {{ $t('workflow.quickActionsRuntime') }}
                  </div>
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
                    v-if="showPlanningModeToggle && planningMode"
                    command="autoApprovePlan"
                    :disabled="!canToggleAutoApprovePlan"
                    :class="{ active: autoApprovePlan }">
                    <cs name="check-circle" size="14px" class="dropdown-icon" />
                    <span class="dropdown-content">
                      <span class="dropdown-main">
                        <span class="dropdown-text">{{ $t('workflow.autoApprovePlan') }}</span>
                        <cs v-if="autoApprovePlan" name="check" size="14px" class="dropdown-check" />
                      </span>
                      <span class="dropdown-note">{{ $t('workflow.autoApprovePlanTooltip') }}</span>
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

            <!-- sandbox -->
            <el-popover
              v-model:visible="sandboxPopoverVisible"
              placement="top"
              :width="360"
              trigger="click"
              popper-class="workflow-sandbox-popover">
              <template #reference>
                <label class="icon-btn upperLayer sandbox-mode-badge" :class="{ active: sandboxMode !== 'host_only' }">
                  <cs name="sandbox" class="small" />
                  <span class="badge">{{ sandboxModeBadge }}</span>
                </label>
              </template>
              <div v-if="sandboxPopoverVisible" class="workflow-sandbox-panel">
                <div class="sandbox-section-title">{{ $t('settings.agent.sandboxExecutionMode') }}</div>
                <button
                  v-for="option in sandboxModeOptions"
                  :key="option.value"
                  type="button"
                  class="sandbox-option"
                  :class="{ active: sandboxMode === option.value }"
                  :disabled="option.disabled || isUpdatingSandboxConfig"
                  @click="selectSandboxMode(option.value)">
                  <span class="sandbox-option-copy">
                    <span>{{ $t(option.label) }}</span>
                  </span>
                  <cs v-if="sandboxMode === option.value" name="check" size="14px" class="dropdown-check" />
                </button>

                <template v-if="sandboxMode !== 'host_only'">
                  <div class="section-divider"></div>
                  <div class="sandbox-section-title">{{ $t('settings.agent.sandboxConfig') }}</div>
                  <button
                    v-for="scheme in selectableSandboxSchemes"
                    :key="scheme.id"
                    type="button"
                    class="sandbox-option"
                    :class="{ active: sandboxSchemeId === scheme.id }"
                    :disabled="isUpdatingSandboxConfig"
                    @click="selectSandboxScheme(scheme.id)">
                    <span class="sandbox-option-copy">
                      <span>{{ scheme.name }}</span>
                    </span>
                    <cs
                      v-if="sandboxSchemeId === scheme.id"
                      name="check"
                      size="14px"
                      class="dropdown-check" />
                  </button>
                  <div v-if="selectableSandboxSchemes.length === 0" class="sandbox-empty">
                    {{ $t('settings.agent.sandboxProfilesEmpty') }}
                  </div>
                </template>
              </div>
            </el-popover>

            <!-- Approval Level Dropdown -->
            <el-dropdown trigger="click" @command="$emit('update-approval-level', $event)">
              <label
                class="icon-btn upperLayer"
                :class="{ 'warning-mode': approvalLevel === 'full' }">
                <cs
                  :key="approvalLevel"
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
              v-model:visible="autoApprovedPopoverVisible"
              placement="top"
              :width="400"
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

              <div
                v-if="autoApprovedPopoverVisible"
                class="auto-approved-panel"
                @click.stop
                @mousedown.stop>
                <el-tabs v-model="approvalToolsTab" class="approval-tools-tabs">
                  <el-tab-pane :label="`${$t('settings.agent.availableTools')} (${agentAvailableTools.length})`" name="available">
                    <div v-if="agentAvailableTools.length > 0" class="section-content checkbox-list">
                      <label v-for="tool in agentAvailableTools" :key="tool.id" class="checkbox-item tool-checkbox-item">
                        <el-checkbox
                          :model-value="workflowAvailableToolIds.includes(tool.id)"
                          @change="checked => toggleWorkflowAvailableTool(tool.id, checked)">
                          <span class="checkbox-label-wrap">
                            <code class="tool-name">{{ tool.id }}</code>
                            <span v-if="tool.name && tool.name !== tool.id" class="tool-desc">{{ tool.name }}</span>
                          </span>
                        </el-checkbox>
                      </label>
                    </div>
                    <div v-else class="section-empty-text">{{ $t('common.noData') }}</div>
                  </el-tab-pane>
                  <el-tab-pane :label="`${$t('workflow.autoApprovedTools')} (${autoApprovedTools.length})`" name="autoApprove">
                    <div v-if="availableApprovalTools.length > 0" class="section-content checkbox-list">
                      <label v-for="tool in availableApprovalTools" :key="tool.id" class="checkbox-item tool-checkbox-item">
                        <el-checkbox :model-value="autoApprovedTools.includes(tool.id)" @change="checked => toggleAutoApprovedTool(tool.id, checked)">
                          <span class="checkbox-label-wrap">
                            <code class="tool-name">{{ tool.id }}</code>
                            <span v-if="tool.name && tool.name !== tool.id" class="tool-desc">{{ tool.name }}</span>
                          </span>
                        </el-checkbox>
                      </label>
                    </div>
                    <div v-else class="section-empty-text">{{ $t('common.noData') }}</div>
                  </el-tab-pane>
                  <el-tab-pane :label="`${$t('workflow.allowedShellCommands')} (${shellPolicyRules.length})`" name="shell">
                    <div class="panel-section">
                  <div class="section-toolbar shell-policy-search">
                    <el-input
                      v-model="shellCommandSearch"
                      size="small"
                      clearable
                      :placeholder="$t('common.search') || 'Search shell command pattern'" />
                  </div>
                  <div class="section-toolbar shell-policy-add-row">
                    <el-input
                      v-model="newShellCommandPattern"
                      size="small"
                      clearable
                      :placeholder="
                        $t('settings.agent.shellPolicyPattern') || 'Enter shell command pattern'
                      "
                      @keydown.enter.prevent="addShellPolicyItem" />
                    <el-radio-group v-model="newShellCommandDecision" class="shell-policy-decision-group">
                      <el-radio-button value="allow">{{ $t('settings.agent.shellDecisionAllow') }}</el-radio-button>
                      <el-radio-button value="deny">{{ $t('settings.agent.shellDecisionDeny') }}</el-radio-button>
                      <el-radio-button value="review">{{ $t('settings.agent.shellDecisionReview') }}</el-radio-button>
                    </el-radio-group>
                    <el-button
                      size="small"
                      type="primary"
                      :disabled="!canAddShellPolicyItem"
                      @click="addShellPolicyItem">
                      {{ $t('settings.agent.shellPolicyAdd') || 'Add' }}
                    </el-button>
                  </div>
                  <template v-if="filteredShellPolicyRules.length > 0">
                    <div class="section-content">
                      <div
                        v-for="(rule, idx) in paginatedShellPolicyRules"
                        :key="`${rule.pattern}-${idx}`"
                        class="tool-item shell-item">
                        <div class="tool-info">
                        <code class="tool-name shell-pattern">{{ rule.pattern }}</code>
                        <span v-if="rule.description" class="tool-desc">{{ rule.description }}</span>
                      </div>
                      <el-radio-group
                        :model-value="rule.decision || 'review'"
                        class="shell-policy-decision-group shell-policy-item-decision"
                        @change="decision => updateShellPolicyDecision(rule.pattern, decision)">
                        <el-radio-button value="allow">{{ $t('settings.agent.shellDecisionAllow') }}</el-radio-button>
                        <el-radio-button value="deny">{{ $t('settings.agent.shellDecisionDeny') }}</el-radio-button>
                        <el-radio-button value="review">{{ $t('settings.agent.shellDecisionReview') }}</el-radio-button>
                      </el-radio-group>
                      <el-button
                        size="small"
                        type="danger"
                        text
                        class="remove-btn"
                        @click="removeShellPolicyItem(rule.pattern)">
                        <cs name="trash" size="12px" />
                      </el-button>
                    </div>
                  </div>
                  <el-pagination
                    v-if="filteredShellPolicyRules.length > SHELL_POLICY_PAGE_SIZE"
                    v-model:current-page="shellPolicyPage"
                    class="shell-policy-pagination"
                    :page-size="SHELL_POLICY_PAGE_SIZE"
                    :total="filteredShellPolicyRules.length"
                    layout="prev, pager, next"
                    size="small"
                    background />
                  </template>
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
                    <div class="section-footer-actions">
                      <el-tooltip
                        placement="top"
                        :content="$t('settings.agent.shellPolicyImportAgent')"
                        :hide-after="0"
                        :enterable="false">
                        <button
                          type="button"
                          class="section-footer-action"
                          :disabled="isImportingShellPolicies || !currentWorkflowId || !selectedAgent?.id"
                          @click="importAgentShellPolicies">
                          <cs name="import" size="12px" />
                        </button>
                      </el-tooltip>
                      <el-tooltip
                        placement="top"
                        :content="$t('settings.agent.shellPolicyClear')"
                        :hide-after="0"
                        :enterable="false">
                        <button
                          type="button"
                          class="section-footer-action clear-shell-policy-action"
                          :disabled="isClearingShellPolicies || !currentWorkflowId || shellPolicyRules.length === 0"
                          @click="clearShellPolicyRules">
                          <cs name="trash" size="12px" />
                        </button>
                      </el-tooltip>
                    </div>
                  </div>
                    </div>
                  </el-tab-pane>
                </el-tabs>
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
          <cs name="stop" @click="confirmStop" v-if="canStop" />
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
import { ref, computed, watch, nextTick } from 'vue'
import { ElMessageBox } from 'element-plus'
import { useI18n } from 'vue-i18n'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'
import { useSandboxSchemeStore } from '@/stores/sandbox_scheme'
import { createLatestModelConfigSaver } from '@/composables/workflow/modelConfigPersistence'
import {
  buildCurrentModelOptions,
  getModelConfigForOption,
  resolveActiveModelConfig
} from '@/composables/workflow/modelConfigSelection'
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
  saveModelConfig: {
    type: Function,
    required: true
  },
  showPlanningModeToggle: {
    type: Boolean,
    default: true
  },
  planningMode: {
    type: Boolean,
    default: false
  },
  autoApprovePlan: {
    type: Boolean,
    default: false
  },
  canToggleAutoApprovePlan: {
    type: Boolean,
    default: true
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
    default: false
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
  'toggle-auto-approve-plan',
  'toggle-final-audit-mode',
  'toggle-auto-compress',
  'trigger-manual-compress',
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
const modelStore = useModelStore()
const settingStore = useSettingStore()
const sandboxSchemeStore = useSandboxSchemeStore()
const modelSelectorOpen = ref(false)
const modelSelectorRef = ref(null)
const modelConfigDraft = ref(null)

const THINKING_LEVEL_TO_BUDGET = {
  low: 1024,
  medium: 2048,
  high: 4096,
  max: 8192
}
const thinkingLevelOptions = [
  { value: 'low', label: 'settings.model.reasoningLow' },
  { value: 'medium', label: 'settings.model.reasoningMedium' },
  { value: 'high', label: 'settings.model.reasoningHigh' },
  { value: 'max', label: 'settings.model.reasoningMax' }
]

const defaultModelConfig = () => ({
  id: '',
  model: '',
  temperature: -0.1,
  thinking: { type: 'disabled' },
  contextSize: 128000,
  maxTokens: 0
})
const cloneModelConfig = model => ({ ...defaultModelConfig(), ...(model || {}) })
const getAgentModelConfigs = () => ({
  plan: props.selectedAgent?.planModel,
  act: props.selectedAgent?.actModel,
  utility: props.selectedAgent?.utilityModel,
  vision: props.selectedAgent?.visionModel
})
const sourceModelConfigs = computed(() => {
  const models = props.currentWorkflow?.agentConfig?.models || getAgentModelConfigs()
  return {
    plan: cloneModelConfig(models.plan),
    act: cloneModelConfig(models.act),
    utility: cloneModelConfig(models.utility),
    vision: cloneModelConfig(models.vision)
  }
})
const effectiveModelConfigs = computed(() => modelConfigDraft.value || sourceModelConfigs.value)
const modelConfigScope = computed(() =>
  props.currentWorkflowId ? `workflow:${props.currentWorkflowId}` : `agent:${props.selectedAgent?.id || ''}`
)
const modelConfigActivation = ref(0)
const modelConfigTarget = computed(() =>
  props.currentWorkflowId
    ? { type: 'workflow', sessionId: props.currentWorkflowId }
    : { type: 'agent', agentId: props.selectedAgent?.id || '' }
)
const activeModelKey = computed(() => (props.planningMode ? 'plan' : 'act'))
const activeModelConfig = computed(() =>
  resolveActiveModelConfig(effectiveModelConfigs.value, props.planningMode)
)
const modelOptions = computed(() =>
  buildCurrentModelOptions({
    currentConfig: activeModelConfig.value,
    modelStore,
    settings: settingStore.settings
  })
)
const updateActiveModelConfigs = configs => {
  const scope = modelConfigScope.value
  const activation = modelConfigActivation.value
  modelConfigDraft.value = configs
  latestModelConfigSaver.submit({
    scope,
    activation,
    target: { ...modelConfigTarget.value },
    configs
  })
}
const latestModelConfigSaver = createLatestModelConfigSaver({
  save: submission => props.saveModelConfig(submission),
  onSuccess: submission => {
    if (
      submission.scope === modelConfigScope.value &&
      submission.activation === modelConfigActivation.value
    ) {
      modelConfigDraft.value = null
    }
  },
  onFailure: submission => {
    if (
      submission.scope === modelConfigScope.value &&
      submission.activation === modelConfigActivation.value
    ) {
      modelConfigDraft.value = null
    }
  }
})
const selectModel = option => {
  const configs = { ...effectiveModelConfigs.value }
  configs[activeModelKey.value] = getModelConfigForOption(option, activeModelConfig.value)
  updateActiveModelConfigs(configs)
}
const updateThinkingLevel = (option, level) => {
  const configs = { ...effectiveModelConfigs.value }
  const currentConfig = activeModelConfig.value
  const selectedConfig =
    currentConfig.id === option.id && currentConfig.model === option.model
      ? { ...currentConfig }
      : getModelConfigForOption(option, currentConfig)
  selectedConfig.thinking = {
    type: 'enabled',
    budgetTokens: THINKING_LEVEL_TO_BUDGET[level] || THINKING_LEVEL_TO_BUDGET.low
  }
  configs[activeModelKey.value] = selectedConfig
  updateActiveModelConfigs(configs)
}

watch(
  modelSelectorOpen,
  async (isOpen, _, onCleanup) => {
    if (!isOpen) return

    const closeOnEscape = event => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      modelSelectorOpen.value = false
    }
    document.addEventListener('keydown', closeOnEscape)
    onCleanup(() => document.removeEventListener('keydown', closeOnEscape))

    await nextTick()
    modelSelectorRef.value
      ?.querySelector('.model-option.active')
      ?.scrollIntoView({ block: 'nearest' })
  }
)

watch(
  modelConfigScope,
  scope => {
    modelConfigActivation.value = latestModelConfigSaver.invalidateScope(scope)
    modelConfigDraft.value = null
  },
  { immediate: true }
)

const autoApprovedPopoverVisible = ref(false)
const approvalToolsTab = ref('available')
const isImportingShellPolicies = ref(false)
const isClearingShellPolicies = ref(false)
const newShellCommandPattern = ref('')
const newShellCommandDecision = ref('review')
const shellCommandSearch = ref('')
const shellPolicyPage = ref(1)
const SHELL_POLICY_PAGE_SIZE = 10

const agentAvailableTools = computed(() => {
  const agentToolIds = Array.isArray(props.selectedAgent?.availableTools)
    ? props.selectedAgent.availableTools
    : []
  const toolDetails = new Map(agentStore.availableTools.map(tool => [tool.id, tool]))

  return agentToolIds
    .map(id => ({ id, name: toolDetails.get(id)?.name || id }))
    .sort((a, b) => a.id.localeCompare(b.id, 'zh-Hans'))
})

const workflowAvailableToolIds = computed(() => {
  if (Array.isArray(props.currentWorkflow?.agentConfig?.availableTools)) {
    return props.currentWorkflow.agentConfig.availableTools
  }
  return Array.isArray(props.selectedAgent?.availableTools) ? props.selectedAgent.availableTools : []
})
const autoApprovedTools = computed(() => {
  const availableSet = new Set(workflowAvailableToolIds.value)
  return workflowStore.autoApprovedTools
    .filter(tool => availableSet.has(tool))
    .sort((a, b) => a.localeCompare(b))
})
const shellPolicyRules = computed(() => {
  const policy = Array.isArray(props.currentWorkflow?.agentConfig?.shellPolicy)
    ? props.currentWorkflow.agentConfig.shellPolicy
    : Array.isArray(props.currentWorkflow?.shellPolicy)
      ? props.currentWorkflow.shellPolicy
      : []

  return policy
    .filter(rule => rule && rule.pattern)
    .map(rule => ({ ...rule, decision: rule.decision || 'review' }))
    .sort((a, b) => String(a.pattern).localeCompare(String(b.pattern)))
})
const allowedShellCommands = computed(() =>
  shellPolicyRules.value.filter(rule => rule.decision === 'allow')
)
const filteredShellPolicyRules = computed(() => {
  const keyword = shellCommandSearch.value.trim().toLowerCase()
  if (!keyword) return shellPolicyRules.value

  return shellPolicyRules.value.filter(rule => {
    const pattern = String(rule.pattern || '').toLowerCase()
    const description = String(rule.description || '').toLowerCase()
    return pattern.includes(keyword) || description.includes(keyword)
  })
})
const paginatedShellPolicyRules = computed(() => {
  const pageCount = Math.max(
    1,
    Math.ceil(filteredShellPolicyRules.value.length / SHELL_POLICY_PAGE_SIZE)
  )
  const currentPage = Math.min(shellPolicyPage.value, pageCount)
  const start = (currentPage - 1) * SHELL_POLICY_PAGE_SIZE
  return filteredShellPolicyRules.value.slice(start, start + SHELL_POLICY_PAGE_SIZE)
})
watch(shellCommandSearch, () => {
  shellPolicyPage.value = 1
})
watch(
  () => filteredShellPolicyRules.value.length,
  count => {
    const lastPage = Math.max(1, Math.ceil(count / SHELL_POLICY_PAGE_SIZE))
    if (shellPolicyPage.value > lastPage) shellPolicyPage.value = lastPage
  }
)
const availableApprovalTools = computed(() => {
  const allowedSet = new Set(
    workflowAvailableToolIds.value.filter(
      toolId => toolId && toolId !== 'bash' && toolId !== 'mcp_tool_load'
    )
  )

  return agentAvailableTools.value
    .filter(tool => allowedSet.has(tool.id))
    .filter(tool => tool.id !== 'bash' && tool.id !== 'mcp_tool_load')
    .sort((a, b) => a.id.localeCompare(b.id, 'zh-Hans'))
})
const autoApprovedItemCount = computed(
  () => autoApprovedTools.value.length + allowedShellCommands.value.length
)
const canAddShellPolicyItem = computed(() =>
  Boolean(props.currentWorkflowId && newShellCommandPattern.value.trim())
)
const sandboxPopoverVisible = ref(false)
const isUpdatingSandboxConfig = ref(false)
const sandboxMode = computed(
  () => props.currentWorkflow?.agentConfig?.sandboxExecutionMode || props.selectedAgent?.sandboxExecutionMode || 'host_only'
)
const sandboxModeBadge = computed(() => ({ auto: 'A', sandbox_only: 'S', host_only: 'H' })[sandboxMode.value] || 'H')
const sandboxSchemeId = computed(
  () => props.currentWorkflow?.agentConfig?.sandboxSchemeId || props.selectedAgent?.sandboxSchemeId || null
)
const enabledSandboxSchemes = computed(() =>
  sandboxSchemeStore.schemes.filter(
    scheme => !scheme.disabled && (scheme.config?.profiles || []).some(profile => profile.enabled)
  )
)
const selectableSandboxSchemes = computed(() => {
  if (sandboxMode.value !== 'auto') return enabledSandboxSchemes.value
  return enabledSandboxSchemes.value.filter(scheme =>
    (scheme.config?.profiles || []).some(
      profile => profile.enabled && (profile.commandPatterns || []).every(pattern => /^\^?\.\*\$?$/.test(pattern.trim()))
    )
  )
})
const sandboxModeOptions = computed(() => {
  const hasSandboxConfig = selectableSandboxSchemes.value.length > 0
  return [
    { value: 'auto', label: 'settings.agent.sandboxExecutionModeAuto', disabled: !hasSandboxConfig },
    {
      value: 'sandbox_only',
      label: 'settings.agent.sandboxExecutionModeSandboxOnly',
      disabled: !enabledSandboxSchemes.value.length
    },
    { value: 'host_only', label: 'settings.agent.sandboxExecutionModeHostOnly', disabled: false }
  ]
})

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

const persistSandboxConfig = async (executionMode, schemeId) => {
  if (!props.currentWorkflowId || isUpdatingSandboxConfig.value) return false

  isUpdatingSandboxConfig.value = true
  try {
    await invokeWrapper('update_workflow_sandbox_config', {
      sessionId: props.currentWorkflowId,
      executionMode,
      sandboxSchemeId: schemeId
    })
    if (props.currentWorkflow?.agentConfig) {
      props.currentWorkflow.agentConfig = {
        ...props.currentWorkflow.agentConfig,
        sandboxOverride: true,
        sandboxExecutionMode: executionMode,
        sandboxSchemeId: schemeId
      }
    }
    return true
  } catch (error) {
    console.error('Failed to update workflow sandbox configuration:', error)
    showMessage(error?.message || t('settings.agent.sandboxConfigTip'), 'error')
    return false
  } finally {
    isUpdatingSandboxConfig.value = false
  }
}

const selectSandboxMode = async executionMode => {
  if (executionMode === sandboxMode.value) return

  const schemes = executionMode === 'auto' ? selectableSandboxSchemes.value : enabledSandboxSchemes.value
  const schemeId = executionMode === 'host_only'
    ? null
    : schemes.some(scheme => scheme.id === sandboxSchemeId.value)
      ? sandboxSchemeId.value
      : schemes[0]?.id

  if (executionMode !== 'host_only' && !schemeId) return
  await persistSandboxConfig(executionMode, schemeId)
}

const selectSandboxScheme = async schemeId => {
  if (!schemeId || schemeId === sandboxSchemeId.value) return
  await persistSandboxConfig(sandboxMode.value, schemeId)
}

watch(sandboxPopoverVisible, async isVisible => {
  if (!isVisible || sandboxSchemeStore.loading) return
  try {
    await sandboxSchemeStore.fetchSchemes()
  } catch (error) {
    console.error('Failed to load sandbox schemes:', error)
  }
})

const toggleWorkflowAvailableTool = async (toolId, checked) => {
  if (!props.currentWorkflowId) return

  const currentTools = workflowAvailableToolIds.value
  const nextAvailableTools = checked
    ? [...new Set([...currentTools, toolId])]
    : currentTools.filter(id => id !== toolId)
  const nextAutoApprove = checked
    ? autoApprovedTools.value
    : autoApprovedTools.value.filter(id => id !== toolId)

  try {
    await persistAgentConfig({
      availableTools: nextAvailableTools,
      autoApprove: nextAutoApprove
    })
    workflowStore.setAutoApprovedTools(nextAutoApprove)
  } catch (error) {
    console.error('Failed to update workflow available tools:', error)
  }
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

const getCurrentShellPolicy = () =>
  Array.isArray(props.currentWorkflow?.agentConfig?.shellPolicy)
    ? props.currentWorkflow.agentConfig.shellPolicy
    : Array.isArray(props.currentWorkflow?.shellPolicy)
      ? props.currentWorkflow.shellPolicy
      : []

const saveShellPolicy = async nextPolicy => {
  await persistAgentConfig({ shellPolicy: nextPolicy })
  workflowStore.setShellPolicy(nextPolicy)
}

const removeShellPolicyItem = async pattern => {
  const nextPolicy = getCurrentShellPolicy().filter(rule => rule.pattern !== pattern)
  try {
    await saveShellPolicy(nextPolicy)
  } catch (error) {
    console.error('Failed to remove shell policy item:', error)
  }
}

const updateShellPolicyDecision = async (pattern, decision) => {
  const nextPolicy = getCurrentShellPolicy().map(rule =>
    rule.pattern === pattern ? { ...rule, decision } : rule
  )
  try {
    await saveShellPolicy(nextPolicy)
  } catch (error) {
    console.error('Failed to update shell policy decision:', error)
  }
}

const addShellPolicyItem = async () => {
  const pattern = newShellCommandPattern.value.trim()
  if (!props.currentWorkflowId || !pattern) return

  const currentPolicy = getCurrentShellPolicy()
  if (currentPolicy.some(rule => rule.pattern === pattern)) {
    showMessage(t('settings.agent.shellPolicyDuplicate'), 'info')
    return
  }

  try {
    await saveShellPolicy([
      ...currentPolicy,
      { pattern, decision: newShellCommandDecision.value }
    ])
    newShellCommandPattern.value = ''
  } catch (error) {
    console.error('Failed to add shell policy item:', error)
  }
}

const clearShellPolicyRules = async () => {
  if (!props.currentWorkflowId || isClearingShellPolicies.value) return

  try {
    await ElMessageBox.confirm(
      t('settings.agent.shellPolicyClearConfirm'),
      t('settings.agent.shellPolicyClearTitle'),
      {
        confirmButtonText: t('common.confirm'),
        cancelButtonText: t('common.cancel'),
        type: 'warning'
      }
    )
    isClearingShellPolicies.value = true
    await saveShellPolicy([])
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') {
      console.error('Failed to clear shell policy:', error)
    }
  } finally {
    isClearingShellPolicies.value = false
  }
}

const importAgentShellPolicies = async () => {
  if (!props.currentWorkflowId || !props.selectedAgent?.id || isImportingShellPolicies.value) return

  try {
    await ElMessageBox.confirm(
      t('settings.agent.shellPolicyImportAgentConfirm'),
      t('settings.agent.shellPolicyImportAgentTitle'),
      {
        confirmButtonText: t('common.confirm'),
        cancelButtonText: t('common.cancel'),
        type: 'info'
      }
    )
    isImportingShellPolicies.value = true
    const agentPolicy = Array.isArray(props.selectedAgent.shellPolicy)
      ? props.selectedAgent.shellPolicy
      : []
    await saveShellPolicy(agentPolicy.map(rule => ({ ...rule })))
    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    if (error !== 'cancel' && error !== 'close') {
      console.error('Failed to import agent shell policy:', error)
    }
  } finally {
    isImportingShellPolicies.value = false
  }
}


const inputRef = ref(null)
const quickActionsDropdownRef = ref(null)
const createWorkflowDialogVisible = ref(false)
const createWorkflowInheritCurrent = ref(true)

const inputMessage = defineModel('inputMessage', { type: String, default: '' })
const isInputExpanded = ref(false)

watch(inputMessage, value => {
  if (value.trim() === '') {
    isInputExpanded.value = false
  }
})

const autoCompressMenuLabel = computed(() => t('workflow.autoCompressShort'))
const activeRuntimeOptionCount = computed(
  () =>
    Number(props.planningMode) +
    Number(props.planningMode && props.autoApprovePlan) +
    Number(props.finalAuditMode !== 'off') +
    Number(props.autoCompressEnabled)
)

const openCreateWorkflowDialog = () => {
  createWorkflowInheritCurrent.value = Boolean(props.currentWorkflow)
  createWorkflowDialogVisible.value = true
}

const createWorkflowFromSelectedMode = () => {
  createWorkflowDialogVisible.value = false
  emit('create-new-workflow', { inheritCurrent: createWorkflowInheritCurrent.value })
}

const confirmStop = async () => {
  try {
    await ElMessageBox.confirm(t('workflow.stopConfirmMessage'), t('workflow.stopConfirmTitle'), {
      confirmButtonText: t('workflow.stop'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    })
    emit('stop')
  } catch {
    // Cancelling leaves the workflow running.
  }
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
    quickActionsDropdownRef.value?.handleClose?.()
    emit('open-image-dialog')
    return
  }

  if (command === 'manualCompress') {
    quickActionsDropdownRef.value?.handleClose?.()
    emit('trigger-manual-compress')
    return
  }

  if (command === 'skillsConfig') {
    quickActionsDropdownRef.value?.handleClose?.()
    emit('open-skills-selector')
    return
  }

  if (command === 'modelConfig') {
    quickActionsDropdownRef.value?.handleClose?.()
    emit('open-model-selector')
    return
  }

  if (command === 'planning') {
    if (props.showPlanningModeToggle && props.canTogglePlanningMode) {
      emit('toggle-planning-mode')
    }
    return
  }

  if (command === 'autoApprovePlan') {
    if (props.planningMode && props.canToggleAutoApprovePlan) {
      emit('toggle-auto-approve-plan')
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
  background: var(--cs-fill-color-light);
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
  color: var(--cs-text-color-secondary);
}

.workflow-attachment-remove {
  cursor: pointer;
  flex-shrink: 0;
  color: var(--cs-text-color-secondary);
}

.approval-tools-tabs {
  display: flex;
  flex-direction: column;
  height: 360px;

  :deep(.el-tabs__header) {
    flex: 0 0 auto;
    margin: 0 0 var(--cs-space-sm);
  }

  :deep(.el-tabs__content) {
    flex: 1;
    min-height: 0;
  }

  :deep(.el-tab-pane) {
    height: 100%;
    overflow-y: auto;
    padding-right: var(--cs-space-xs);
  }

  :deep(.el-tabs__item) {
    min-width: 0;
    padding: 0 var(--cs-space-sm);
    font-size: var(--cs-font-size-xs);

    &:nth-child(2){
        padding-left: var(--cs-space-sm);
    }
  }
}

.shell-policy-pagination {
  justify-content: center;
  margin-top: var(--cs-space-sm);
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

.shell-policy-search {
  margin-bottom: 8px;
}

.shell-policy-add-row {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 10px;

  :deep(.el-input) {
    flex: 1;
    min-width: 0;
  }
}

.shell-policy-decision-group {
  display: inline-flex;
  flex-shrink: 0;

  :deep(.el-radio-button__inner) {
    padding: 5px 7px;
    font-size: var(--cs-font-size-xs);
  }
}

.shell-policy-item-decision {
  margin-left: auto;
}

.section-footer-actions {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.clear-shell-policy-action:hover:not(:disabled) {
  color: var(--el-color-danger);
}

.section-toolbar {
  margin-bottom: 10px;
}

.section-empty-text {
  font-size: 12px;
  color: var(--cs-text-color-secondary);
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
  color: var(--cs-text-color-secondary);
  cursor: pointer;
  transition:
    background-color 0.2s ease,
    color 0.2s ease;
}

.section-footer-action:hover:not(:disabled) {
  background: var(--cs-fill-color-light);
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
  margin: 1px 0;
  padding: var(--cs-space-xs) var(--cs-space-sm);
  border-radius: var(--cs-border-radius);
  line-height: 1.35;
  transition:
    background-color 0.16s ease,
    color 0.16s ease;

  &:hover,
  &:focus-visible {
    background: var(--cs-hover-bg-color);
  }

  &.active {
    background: var(--el-color-primary-light-9);

    .dropdown-icon,
    .dropdown-check {
      color: var(--cs-color-primary);
    }
  }
}

.quick-actions-divider {
  height: 1px;
  margin: var(--cs-space-xs) var(--cs-space-xs);
  background: var(--cs-border-color);
}

.quick-actions-section-title {
  padding: var(--cs-space-xs) var(--cs-space-sm) var(--cs-space-xxs);
  color: var(--cs-text-color-secondary);
  font-size: var(--cs-font-size-xs);
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.dropdown-icon {
  flex-shrink: 0;
  color: var(--cs-text-color-secondary);
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
  gap: var(--cs-space-xs);
  min-width: 0;
}

.dropdown-text {
  min-width: 0;
  line-height: 1.4;
  color: var(--cs-text-color-primary);
}

.dropdown-note {
  margin-top: 2px;
  font-size: var(--cs-font-size-xs);
  line-height: 1.3;
  color: var(--cs-text-color-secondary);
  white-space: normal;
}

.dropdown-check {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--cs-color-primary);
}

.quick-actions-badge {
  .badge {
    position: absolute;
    top: -6px;
    right: -6px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: var(--cs-border-radius-lg);
    background: var(--el-color-primary);
    color: var(--cs-text-color-on-primary);
    font-size: 10px;
    font-weight: 600;
    line-height: 16px;
    text-align: center;
  }

  &.has-active-options .cs {
    color: var(--el-color-primary);
  }
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
