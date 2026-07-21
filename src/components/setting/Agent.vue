<template>
  <div class="card">
    <div class="title">
      <span>{{ $t('settings.agent.title') }}</span>
      <el-tooltip :content="$t('settings.agent.add')" placement="left">
        <span class="icon" @click="editAgent()">
          <cs name="add" />
        </span>
      </el-tooltip>
    </div>
    <Sortable
      v-if="groupedPrimaryAgents.length > 0"
      class="list agent-group-list"
      item-key="id"
      :list="groupedPrimaryAgents"
      :options="{
        animation: 150,
        ghostClass: 'ghost',
        dragClass: 'drag',
        draggable: '.agent-group',
        forceFallback: true,
        bubbleScroll: true
      }"
      @update="onPrimarySortUpdate"
      @end="onPrimaryDragEnd">
      <template #item="{ element }">
        <div class="agent-group" :key="element.id">
          <div class="item draggable">
            <div class="label">
              <avatar :text="element.name" :size="20" />
              {{ element.name }}
            </div>

            <div class="value">
              <el-tooltip
                :content="$t('settings.agent.' + (element.disabled ? 'enable' : 'disable'))"
                placement="top"
                :hide-after="0"
                :enterable="false"
                transition="none">
                <el-switch
                  :model-value="!element.disabled"
                  @update:model-value="toggleAgentStatus(element)" />
              </el-tooltip>
              <el-tooltip
                :content="$t('settings.agent.edit')"
                placement="top"
                :hide-after="0"
                :enterable="false"
                transition="none">
                <div class="icon" @click="editAgent(element.id)" @mousedown.stop>
                  <cs name="edit" size="16px" color="secondary" />
                </div>
              </el-tooltip>
              <el-tooltip
                :content="$t('settings.agent.copy')"
                placement="top"
                :hide-after="0"
                :enterable="false"
                transition="none">
                <div class="icon" @click="copyAgent(element.id)" @mousedown.stop>
                  <cs name="copy" size="16px" color="secondary" />
                </div>
              </el-tooltip>
              <el-tooltip
                v-if="!element.isSystem"
                :content="$t('settings.agent.delete')"
                placement="top"
                :hide-after="0"
                :enterable="false"
                transition="none">
                <div class="icon" @click="deleteAgent(element.id)" @mousedown.stop>
                  <cs name="trash" size="16px" color="secondary" />
                </div>
              </el-tooltip>
            </div>
          </div>

          <Sortable
            v-if="groupedChildAgents[element.id]?.length"
            class="agent-child-list"
            item-key="id"
            :list="groupedChildAgents[element.id]"
            :options="{
              animation: 150,
              ghostClass: 'ghost',
              dragClass: 'drag',
              draggable: '.agent-child-item',
              forceFallback: true,
              bubbleScroll: true
            }"
            @update="event => onChildSortUpdate(element.id, event)"
            @end="() => onChildDragEnd(element.id)">
            <template #item="{ element: child }">
              <div class="item draggable item--child agent-child-item" :key="child.id">
                <div class="label label--child">
                  <avatar :text="child.name" :size="18" />
                  {{ child.name }}
                </div>

                <div class="value">
                  <el-tooltip
                    :content="$t('settings.agent.' + (child.disabled ? 'enable' : 'disable'))"
                    placement="top"
                    :hide-after="0"
                    :enterable="false"
                    transition="none">
                    <el-switch
                      :model-value="!child.disabled"
                      @update:model-value="toggleAgentStatus(child)" />
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('settings.agent.edit')"
                    placement="top"
                    :hide-after="0"
                    :enterable="false"
                    transition="none">
                    <div class="icon" @click="editAgent(child.id)" @mousedown.stop>
                      <cs name="edit" size="16px" color="secondary" />
                    </div>
                  </el-tooltip>
                  <el-tooltip
                    :content="$t('settings.agent.copy')"
                    placement="top"
                    :hide-after="0"
                    :enterable="false"
                    transition="none">
                    <div class="icon" @click="copyAgent(child.id)" @mousedown.stop>
                      <cs name="copy" size="16px" color="secondary" />
                    </div>
                  </el-tooltip>
                  <el-tooltip
                    v-if="!child.isSystem"
                    :content="$t('settings.agent.delete')"
                    placement="top"
                    :hide-after="0"
                    :enterable="false"
                    transition="none">
                    <div class="icon" @click="deleteAgent(child.id)" @mousedown.stop>
                      <cs name="trash" size="16px" color="secondary" />
                    </div>
                  </el-tooltip>
                </div>
              </div>
            </template>
          </Sortable>
        </div>
      </template>
    </Sortable>
    <div class="list" v-else>
      <div class="item">
        <div class="label">{{ $t('settings.agent.noAgents') }}</div>
      </div>
    </div>
  </div>

  <el-dialog
    v-model="agentDialogVisible"
    width="640px"
    class="agent-edit-dialog"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    @closed="onAgentDialogClose">
    <el-form :model="agentForm" :rules="agentRules" ref="formRef" label-width="100px">
      <el-tabs v-model="activeTab">
        <el-tab-pane :label="$t('settings.agent.basicInfo')" name="basic">
          <el-form-item :label="$t('settings.agent.name')" prop="name">
            <el-input v-model="agentForm.name" :disabled="isSystemIdentityLocked" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.role')" prop="role">
            <el-select
              v-model="agentForm.role"
              style="width: 100%"
              :disabled="isSystemIdentityLocked">
              <el-option
                v-for="option in AGENT_ROLE_OPTIONS"
                :key="option.value"
                :label="$t(option.labelKey)"
                :value="option.value" />
            </el-select>
          </el-form-item>
          <el-form-item
            v-if="agentForm.role === AGENT_ROLE.CHILD"
            :label="$t('settings.agent.parentAgent')"
            prop="parentAgentId">
            <el-select
              v-model="agentForm.parentAgentId"
              style="width: 100%"
              filterable
              :disabled="isSystemIdentityLocked">
              <el-option
                v-for="agent in primaryAgentOptions"
                :key="agent.id"
                :label="agent.name"
                :value="agent.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.description')" prop="description">
            <el-input
              v-model="agentForm.description"
              type="textarea"
              :rows="2"
              :disabled="isSystemIdentityLocked" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.systemPrompt')" prop="systemPrompt">
            <el-input
              v-model="agentForm.systemPrompt"
              type="textarea"
              :rows="5"
              :disabled="isSystemPromptsLocked" />
          </el-form-item>
          <el-form-item
            v-if="agentForm.role !== AGENT_ROLE.CHILD"
            :label="$t('settings.agent.planningPrompt')"
            prop="planningPrompt">
            <el-input
              v-model="agentForm.planningPrompt"
              type="textarea"
              :rows="5"
              :disabled="isSystemPromptsLocked"
              :placeholder="$t('settings.agent.planningPromptPlaceholder')" />
          </el-form-item>
          <el-form-item
            v-if="agentForm.role !== AGENT_ROLE.CHILD"
            :label="$t('settings.agent.imageRecognitionPrompt')"
            prop="imageRecognitionPrompt">
            <el-input
              v-model="agentForm.imageRecognitionPrompt"
              type="textarea"
              :rows="4"
              :disabled="isSystemPromptsLocked"
              :placeholder="$t('settings.agent.imageRecognitionPromptPlaceholder')" />
          </el-form-item>
          <el-form-item :label="$t('settings.agent.disabled')" prop="disabled">
            <el-switch v-model="agentForm.disabled" />
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.models')" name="models">
          <div class="models-layout" :class="{ 'models-layout--single': modelRoles.length === 1 }">
            <div
              class="model-item-compact"
              :class="{ 'model-item-compact--full': modelRoles.length === 1 }"
              v-for="role in modelRoles"
              :key="role.key">
              <div class="header">
                <span class="title">{{ $t(`settings.agent.${role.key}Model`) }}</span>
                <el-radio-group v-model="modelModes[role.key]" size="small">
                  <el-radio-button value="provider">{{
                    $t('settings.agent.modeProvider')
                  }}</el-radio-button>
                  <el-radio-button value="proxy">{{
                    $t('settings.agent.modeProxy')
                  }}</el-radio-button>
                </el-radio-group>
              </div>
              <div class="body">
                <div class="selectors-row">
                  <template v-if="modelModes[role.key] === 'provider'">
                    <el-select
                      v-model="agentForm[role.key + 'Model'].id"
                      size="small"
                      filterable
                      @change="onModelIdChange(role.key)"
                      style="flex: 1">
                      <el-option
                        v-for="provider in modelStore.getAvailableProviders"
                        :key="provider.id"
                        :label="provider.name"
                        :value="provider.id" />
                    </el-select>
                    <el-select
                      v-model="agentForm[role.key + 'Model'].model"
                      size="small"
                      filterable
                      :disabled="!agentForm[role.key + 'Model'].id"
                      @change="value => onProviderModelChange(role.key, value)"
                      style="flex: 1">
                      <el-option
                        v-for="model in getModelList(role.key)"
                        :key="model.id"
                        :label="model.name || model.id"
                        :value="model.id" />
                    </el-select>
                  </template>
                  <template v-else>
                    <el-select
                      v-model="proxyGroups[role.key]"
                      size="small"
                      filterable
                      @change="onProxyGroupChange(role.key)"
                      style="flex: 1">
                      <el-option
                        v-for="group in proxyGroupStore.list"
                        :key="group.name"
                        :label="group.name"
                        :value="group.name" />
                    </el-select>
                    <el-select
                      v-model="proxyAliases[role.key]"
                      size="small"
                      filterable
                      :disabled="!proxyGroups[role.key]"
                      @change="val => onProxyAliasChange(role.key, val)"
                      style="flex: 1">
                      <el-option
                        v-for="alias in getProxyAliases(proxyGroups[role.key])"
                        :key="alias"
                        :label="alias"
                        :value="alias" />
                    </el-select>
                  </template>
                </div>
                <div class="params-row" style="margin-top: 8px; padding: 0 4px">
                  <span class="param-label">{{ $t('settings.agent.temperature') }}</span>
                  <el-slider
                    v-model="agentForm[role.key + 'Model'].temperature"
                    :min="-0.1"
                    :max="2"
                    :step="0.1"
                    size="small"
                    style="flex: 1; margin-left: 12px" />
                  <span
                    class="param-value"
                    style="font-size: 11px; min-width: 24px; text-align: right"
                    >{{
                      (agentForm[role.key + 'Model']?.temperature ?? -0.1) < 0
                        ? 'Off'
                        : agentForm[role.key + 'Model']?.temperature?.toFixed(1) || '0.0'
                    }}</span
                  >
                </div>
                <div class="params-row compact-params" style="margin-top: 4px">
                  <div class="param-item">
                    <span class="param-label">{{ $t('settings.model.contextSize') }}</span>
                    <el-input-number
                      v-model="agentForm[role.key + 'Model'].contextSize"
                      :min="1024"
                      :max="2000000"
                      :step="1024"
                      size="small"
                      controls-position="right"
                      style="width: 120px" />
                  </div>
                </div>
                <div class="params-row compact-params" style="margin-top: 4px">
                  <div class="param-item">
                    <span class="param-label">{{ $t('settings.model.maxTokens') }}</span>
                    <el-input-number
                      v-model="agentForm[role.key + 'Model'].maxTokens"
                      :min="0"
                      :max="128000"
                      :step="1024"
                      size="small"
                      controls-position="right"
                      style="width: 120px" />
                  </div>
                </div>
                <div
                  v-if="supportsThinking(role.key)"
                  class="params-row compact-params"
                  style="margin-top: 6px">
                  <div class="param-item">
                    <span class="param-label">{{ $t('settings.model.reasoning') }}</span>
                    <el-switch
                      v-model="agentForm[role.key + 'Model'].thinkingEnabled"
                      size="small" />
                  </div>
                  <div class="param-item" v-if="agentForm[role.key + 'Model'].thinkingEnabled">
                    <span class="param-label">{{ $t('settings.model.thinkingLevel') }}</span>
                    <el-select
                      v-model="agentForm[role.key + 'Model'].thinkingLevel"
                      size="small"
                      style="width: 120px">
                      <el-option
                        v-for="option in agentThinkingLevelOptions"
                        :key="option.value"
                        :label="$t(option.label)"
                        :value="option.value" />
                    </el-select>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.skillsLabel')" name="skills">
          <el-form-item :label="$t('settings.agent.skillEnabled')" prop="skillEnabled">
            <el-switch v-model="agentForm.skillEnabled" />
          </el-form-item>
          <el-form-item
            v-if="agentForm.skillEnabled"
            :label="$t('settings.agent.selectedSkills')"
            prop="selectedSkills">
            <el-input
              v-if="sortedSystemSkills.length"
              v-model="skillSearchKeyword"
              clearable
              class="skill-search-input"
              :placeholder="$t('workflow.skillsSearchPlaceholder')" />
            <div v-if="filteredSystemSkills.length" class="skill-checklist">
              <el-checkbox-group v-model="agentForm.selectedSkills" class="skill-checklist__group">
                <label
                  v-for="skill in filteredSystemSkills"
                  :key="skill.name"
                  class="skill-checklist__item">
                  <el-checkbox :value="skill.name">
                    <span class="skill-checklist__name">{{ skill.name }}</span>
                  </el-checkbox>
                  <span
                    v-if="skill.description"
                    class="skill-checklist__description"
                    :title="skill.description">
                    {{ skill.description }}
                  </span>
                </label>
              </el-checkbox-group>
            </div>
            <div class="form-tip">{{ $t('settings.agent.skillsHint') }}</div>
            <div v-if="!sortedSystemSkills.length" class="form-tip">
              {{ $t('settings.agent.noSkillsAvailable') }}
            </div>
            <div v-else-if="!filteredSystemSkills.length" class="form-tip">
              {{ $t('workflow.skillsSearchEmpty') }}
            </div>
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.toolsLabel')" name="tools">
          <el-form-item :label="$t('settings.agent.approvalLevel')" prop="approvalLevel">
            <el-select v-model="agentForm.approvalLevel" style="width: 100%">
              <el-option :label="$t('settings.agent.approvalLevelDefault')" value="default" />
              <el-option :label="$t('settings.agent.approvalLevelSmart')" value="smart" />
              <el-option
                :label="$t('settings.agent.approvalLevelFull')"
                value="full"
                class="danger-option" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.availableTools')" prop="availableTools">
            <el-select
              v-model="agentForm.availableTools"
              :placeholder="$t('settings.agent.selectAvailableTools')"
              multiple
              filterable>
              <el-option
                v-for="tool in sortedAvailableTools"
                :key="tool.id"
                :label="tool.name"
                :value="tool.id" />
            </el-select>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.mcpToolExposure')" prop="mcpToolExposure">
            <el-select
              v-model="agentForm.mcpToolExposure"
              :placeholder="$t('settings.agent.selectMcpToolExposure')"
              multiple
              filterable>
              <el-option
                v-for="tool in availableMcpToolOptions"
                :key="tool.id"
                :label="tool.name"
                :value="tool.id"
                :title="tool.description" />
            </el-select>
            <div class="form-tip">{{ $t('settings.agent.mcpToolExposureTip') }}</div>
          </el-form-item>
          <el-form-item :label="$t('settings.agent.autoApprove')" prop="autoApprove">
            <el-select
              v-model="agentForm.autoApprove"
              :placeholder="$t('settings.agent.selectAutoApproveTools')"
              multiple
              filterable>
              <el-option
                v-for="tool in autoApproveOptions"
                :key="tool.id"
                :label="tool.name"
                :value="tool.id" />
            </el-select>
          </el-form-item>
        </el-tab-pane>

        <el-tab-pane :label="$t('settings.agent.security')" name="security">
          <div class="security-group">
            <div class="shell-policy-header">
              <h3>{{ $t('settings.agent.authorizedPaths') }}</h3>
              <div class="shell-policy-actions">
                <el-button type="primary" size="small" @click="addAuthorizedPath">
                  {{ $t('settings.agent.authorizedPathsAdd') }}
                </el-button>
              </div>
            </div>
            <p class="security-tip">{{ $t('settings.agent.authorizedPathsTip') }}</p>
            <div class="shell-policy-list">
              <div
                v-for="(path, index) in agentForm.allowedPaths"
                :key="index"
                class="shell-policy-item">
                <el-input
                  v-model="agentForm.allowedPaths[index]"
                  size="small"
                  readonly
                  style="flex: 1" />
                <el-button type="danger" size="small" circle @click="removeAuthorizedPath(index)">
                  <cs name="trash" size="12px" />
                </el-button>
              </div>
            </div>
            <div v-if="agentForm.role !== AGENT_ROLE.CHILD" class="security-switch-row">
              <span class="security-switch-label">{{ $t('settings.agent.allowShell') }}</span>
              <el-switch v-model="agentForm.allowShell" />
              <span class="security-switch-tip">{{ $t('settings.agent.allowShellTip') }}</span>
            </div>
          </div>

          <div v-if="canConfigureShellPolicy" class="security-group" style="margin-top: 24px">
            <div class="shell-policy-header">
              <h3>{{ $t('settings.agent.shellPolicy') }}</h3>
              <div class="shell-policy-actions">
                <el-button type="primary" size="small" @click="addShellPolicyRule">
                  {{ $t('settings.agent.shellPolicyAdd') }}
                </el-button>
                <el-button type="info" size="small" @click="importDefaultShellPolicies" plain>
                  {{ $t('settings.agent.shellPolicyImportDefault') }}
                </el-button>
                <el-button
                  v-if="agentForm.shellPolicy && agentForm.shellPolicy.length > 0"
                  type="danger"
                  size="small"
                  @click="clearShellPolicyRules"
                  plain>
                  {{ $t('settings.agent.shellPolicyClear') }}
                </el-button>
              </div>
            </div>
            <div class="shell-policy-list" ref="shellPolicyListRef">
              <div
                v-for="(rule, index) in agentForm.shellPolicy"
                :key="index"
                class="shell-policy-item">
                <el-input
                  v-model="rule.pattern"
                  size="small"
                  :placeholder="$t('settings.agent.shellPolicyPattern')"
                  style="flex: 1" />
                <el-select v-model="rule.decision" size="small" style="width: 130px">
                  <el-option :label="$t('settings.agent.shellDecisionAllow')" value="allow" />
                  <el-option :label="$t('settings.agent.shellDecisionReview')" value="review" />
                  <el-option :label="$t('settings.agent.shellDecisionDeny')" value="deny" />
                </el-select>
                <el-button type="danger" size="small" circle @click="removeShellPolicyRule(index)">
                  <cs name="trash" size="12px" />
                </el-button>
              </div>
            </div>
          </div>
        </el-tab-pane>
      </el-tabs>
    </el-form>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="agentDialogVisible = false">{{ $t('common.cancel') }}</el-button>
        <el-button type="primary" @click="updateAgent">{{ $t('common.save') }}</el-button>
      </span>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, ref, onMounted, reactive, nextTick, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { storeToRefs } from 'pinia'
import { Sortable } from 'sortablejs-vue3'
import { open } from '@tauri-apps/plugin-dialog'

import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { useModelStore } from '@/stores/model'
import { useAgentStore } from '@/stores/agent'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { useSettingStore } from '@/stores/setting'
import { useWorkflowStore } from '@/stores/workflow'
import { AGENT_ROLE, AGENT_ROLE_OPTIONS } from '@/constants/agent'

const { t } = useI18n()

const modelStore = useModelStore()
const agentStore = useAgentStore()
const proxyGroupStore = useProxyGroupStore()
const settingStore = useSettingStore()
const workflowStore = useWorkflowStore()
const { agents, availableTools } = storeToRefs(agentStore)
const ALWAYS_ENABLED_SKILL_NAMES = ['help']

const formRef = ref(null)
const shellPolicyListRef = ref(null)
const agentDialogVisible = ref(false)
const editId = ref(null)
const activeTab = ref('basic')
const systemSkills = ref([])
const skillSearchKeyword = ref('')
const defaultShellPolicies = ref([])
const shouldBackfillSelectedSkills = ref(false)
const groupedPrimaryAgents = ref([])
const groupedChildAgents = ref({})

const allModelRoles = [{ key: 'plan' }, { key: 'act' }, { key: 'vision' }, { key: 'utility' }]

const modelRoles = computed(() => {
  if (agentForm.value.role === AGENT_ROLE.CHILD) {
    return allModelRoles.filter(role => role.key === 'act')
  }
  return allModelRoles
})

const READ_ONLY_TOOLS = ['read_file', 'grep', 'glob', 'web_fetch', 'todo_list', 'list_dir']
const CHILD_ONLY_TOOL_IDS = ['git_diff', 'git_inspect']
const HIDDEN_AGENT_TOOL_IDS = ['bash']
const CORE_MANAGEMENT_TOOLS = [
  'sub_agent_run',
  'sub_agent_output',
  'sub_agent_stop',
  'todo_create',
  'todo_list',
  'todo_update',
  'todo_get',
  'skill',
  'ask_user',
  'complete_workflow_with_summary',
  'submit_plan'
]

const defaultAgentModelConfig = () => ({
  id: '',
  model: '',
  temperature: -0.1,
  thinking: null,
  thinkingEnabled: false,
  thinkingLevel: 'low',
  contextSize: 128000,
  maxTokens: 0
})
const THINKING_LEVEL_TO_BUDGET = {
  low: 1024,
  medium: 2048,
  high: 4096
}
const thinkingLevelFromBudget = budget => {
  const normalized = Number(budget) || 0
  if (normalized > 2048) return 'high'
  if (normalized > 1024) return 'medium'
  return 'low'
}
const budgetFromThinkingLevel = level =>
  THINKING_LEVEL_TO_BUDGET[level] || THINKING_LEVEL_TO_BUDGET.low
const agentThinkingLevelOptions = [
  { value: 'low', label: 'settings.model.reasoningLow' },
  { value: 'medium', label: 'settings.model.reasoningMedium' },
  { value: 'high', label: 'settings.model.reasoningHigh' }
]

const defaultFormData = {
  name: '',
  description: '',
  role: AGENT_ROLE.PRIMARY,
  parentAgentId: null,
  isSystem: false,
  disabled: false,
  systemPrompt: '',
  planningPrompt: '',
  imageRecognitionPrompt: '',
  availableTools: [],
  allowShell: false,
  autoApprove: [],
  skillEnabled: true,
  selectedSkills: [],
  mcpToolExposure: [],
  shellPolicy: [],
  allowedPaths: [],
  planModel: defaultAgentModelConfig(),
  actModel: defaultAgentModelConfig(),
  visionModel: defaultAgentModelConfig(),
  utilityModel: defaultAgentModelConfig(),
  maxContexts: 128000,
  approvalLevel: 'default'
}

const agentForm = ref({ ...defaultFormData })

// Model config temporary state
const modelModes = reactive({
  plan: 'provider',
  act: 'provider',
  vision: 'provider',
  utility: 'provider'
})
const proxyGroups = reactive({ plan: '', act: '', vision: '', utility: '' })
const proxyAliases = reactive({ plan: '', act: '', vision: '', utility: '' })

// Computed property: available tools sorted by name, filtered to exclude core management tools
const sortedAvailableTools = computed(() => {
  return [...availableTools.value]
    .filter(
      t =>
        !CORE_MANAGEMENT_TOOLS.includes(t.id) &&
        !HIDDEN_AGENT_TOOL_IDS.includes(t.id) &&
        (agentForm.value.role === AGENT_ROLE.CHILD || !CHILD_ONLY_TOOL_IDS.includes(t.id))
    )
    .sort((a, b) => {
      return a.name.localeCompare(b.name, 'zh-Hans')
    })
})

// Computed property: auto-approve tool options (filtered and sorted)
const autoApproveOptions = computed(() => {
  if (!agentForm.value || !agentForm.value.availableTools) return []
  return sortedAvailableTools.value.filter(
    t => agentForm.value.availableTools.includes(t.id) && t.id !== 'bash'
  )
})

const availableMcpToolOptions = computed(() => {
  const enabledToolIds = new Set(agentForm.value.availableTools || [])
  return availableTools.value.filter(
    tool => tool.category === 'MCP' && enabledToolIds.has(tool.id)
  )
})

const sortedSystemSkills = computed(() => {
  return [...systemSkills.value]
    .filter(skill => !ALWAYS_ENABLED_SKILL_NAMES.includes(skill.name))
    .sort((a, b) => {
      if (a.source !== b.source) {
        return a.source === 'user' ? -1 : 1
      }
      return a.name.localeCompare(b.name, 'zh-Hans')
    })
})

const filteredSystemSkills = computed(() => {
  const query = skillSearchKeyword.value.trim().toLowerCase()
  if (!query) return sortedSystemSkills.value
  return sortedSystemSkills.value.filter(skill => skill.name.toLowerCase().includes(query))
})

const defaultSelectedSkillNames = computed(() => {
  return sortedSystemSkills.value.map(skill => skill.name)
})

// Tool ID to name mapping
const toolNameMap = computed(() => {
  const map = {}
  availableTools.value.forEach(tool => {
    map[tool.id] = tool.name
  })
  return map
})

const primaryAgentOptions = computed(() => {
  return agents.value.filter(
    agent =>
      (agent.role || AGENT_ROLE.PRIMARY) === AGENT_ROLE.PRIMARY &&
      !agent.disabled &&
      agent.id !== editId.value
  )
})

const compareAgentsByDisplayOrder = (a, b) => {
  const hasSortIndexA = typeof a.sortIndex === 'number'
  const hasSortIndexB = typeof b.sortIndex === 'number'

  if (!hasSortIndexA && !hasSortIndexB) {
    return 0
  }

  const sortIndexA = hasSortIndexA ? a.sortIndex : Number.MAX_SAFE_INTEGER
  const sortIndexB = hasSortIndexB ? b.sortIndex : Number.MAX_SAFE_INTEGER

  if (sortIndexA !== sortIndexB) {
    return sortIndexA - sortIndexB
  }

  return (a.name || '').localeCompare(b.name || '', 'zh-Hans')
}

const syncGroupedAgentLists = () => {
  const primaryAgents = []
  const childGroups = {}

  agents.value.forEach(agent => {
    if ((agent.role || AGENT_ROLE.PRIMARY) === AGENT_ROLE.CHILD) {
      const parentId = agent.parentAgentId
      if (!parentId) {
        return
      }
      if (!childGroups[parentId]) {
        childGroups[parentId] = []
      }
      childGroups[parentId].push({ ...agent })
      return
    }

    primaryAgents.push({ ...agent })
    if (!childGroups[agent.id]) {
      childGroups[agent.id] = []
    }
  })

  primaryAgents.sort(compareAgentsByDisplayOrder)
  Object.keys(childGroups).forEach(parentId => {
    childGroups[parentId].sort(compareAgentsByDisplayOrder)
  })

  groupedPrimaryAgents.value = primaryAgents
  groupedChildAgents.value = childGroups
}

const flattenGroupedAgents = () => {
  const orderedAgents = []
  groupedPrimaryAgents.value.forEach(primary => {
    orderedAgents.push(primary)
    const children = groupedChildAgents.value[primary.id] || []
    children.forEach(child => {
      orderedAgents.push({
        ...child,
        parentAgentId: primary.id
      })
    })
  })
  return orderedAgents
}

const isSystemAgentReadOnly = computed(() => !!editId.value && agentForm.value.isSystem)
const isSystemIdentityLocked = computed(() => isSystemAgentReadOnly.value)
const isSystemPromptsLocked = computed(() => isSystemAgentReadOnly.value)

const canConfigureShellPolicy = computed(
  () => agentForm.value.role !== AGENT_ROLE.CHILD && agentForm.value.allowShell
)

// Function to sort tool IDs by their names
const sortToolIdsByName = toolIds => {
  if (!toolIds || !Array.isArray(toolIds)) return []
  return [...toolIds].sort((a, b) => {
    const nameA = toolNameMap.value[a] || ''
    const nameB = toolNameMap.value[b] || ''
    return nameA.localeCompare(nameB, 'zh-Hans')
  })
}

const sortSkillNamesByName = skillNames => {
  if (!Array.isArray(skillNames)) return []
  const nameSet = new Set(defaultSelectedSkillNames.value)
  return [...new Set(skillNames)]
    .filter(name => nameSet.has(name))
    .sort((a, b) => a.localeCompare(b, 'zh-Hans'))
}

// Watch for availableTools array changes to maintain sorting
watch(
  () => agentForm.value.availableTools,
  newVal => {
    if (!newVal || !Array.isArray(newVal)) return

    const sorted = sortToolIdsByName(newVal)
    // Check if sorting is needed
    let needsSorting = false
    if (sorted.length !== newVal.length) {
      needsSorting = true
    } else {
      for (let i = 0; i < sorted.length; i++) {
        if (sorted[i] !== newVal[i]) {
          needsSorting = true
          break
        }
      }
    }

    if (needsSorting) {
      // Use nextTick to avoid modifying data during render
      nextTick(() => {
        agentForm.value.availableTools = sorted
      })
    }
  },
  { deep: true }
)

// Watch for autoApprove array changes to maintain sorting
watch(
  () => agentForm.value.autoApprove,
  newVal => {
    if (!newVal || !Array.isArray(newVal)) return

    const sorted = sortToolIdsByName(newVal)
    // Check if sorting is needed
    let needsSorting = false
    if (sorted.length !== newVal.length) {
      needsSorting = true
    } else {
      for (let i = 0; i < sorted.length; i++) {
        if (sorted[i] !== newVal[i]) {
          needsSorting = true
          break
        }
      }
    }

    if (needsSorting) {
      // Use nextTick to avoid modifying data during render
      nextTick(() => {
        agentForm.value.autoApprove = sorted
      })
    }
  },
  { deep: true }
)

watch(
  () => agentForm.value.selectedSkills,
  newVal => {
    if (!Array.isArray(newVal)) return

    const sorted = sortSkillNamesByName(newVal)
    let needsSorting = false
    if (sorted.length !== newVal.length) {
      needsSorting = true
    } else {
      for (let i = 0; i < sorted.length; i++) {
        if (sorted[i] !== newVal[i]) {
          needsSorting = true
          break
        }
      }
    }

    if (needsSorting) {
      nextTick(() => {
        agentForm.value.selectedSkills = sorted
      })
    }
  },
  { deep: true }
)

const cloneDefaultShellPolicies = () => defaultShellPolicies.value.map(rule => ({ ...rule }))

const ensureDefaultShellPoliciesLoaded = async () => {
  if (defaultShellPolicies.value.length > 0) return

  try {
    const result = await invokeWrapper('get_default_shell_policy')
    defaultShellPolicies.value = Array.isArray(result) ? result : []
  } catch (error) {
    console.error('Failed to load default shell policy:', error)
    defaultShellPolicies.value = []
  }
}

const addShellPolicyRule = () => {
  if (!agentForm.value.shellPolicy) agentForm.value.shellPolicy = []
  agentForm.value.shellPolicy.push({ pattern: '', decision: 'review' })

  // Use setTimeout to avoid ResizeObserver loop errors
  // Wait for Vue's DOM update to complete
  nextTick(() => {
    // Use requestAnimationFrame to ensure DOM is fully rendered
    requestAnimationFrame(() => {
      if (shellPolicyListRef.value) {
        // Scroll to bottom
        shellPolicyListRef.value.scrollTop = shellPolicyListRef.value.scrollHeight

        // Focus the pattern input field of the last rule
        // Use another microtask to ensure scrolling is complete
        setTimeout(() => {
          const patternInputs = shellPolicyListRef.value.querySelectorAll(
            '.shell-policy-item .el-input:first-child input'
          )
          if (patternInputs.length > 0) {
            const lastPatternInput = patternInputs[patternInputs.length - 1]
            lastPatternInput.focus()
          }
        }, 0)
      }
    })
  })
}

const addAuthorizedPath = async () => {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t('settings.agent.selectDirectory')
    })
    if (selected) {
      if (!agentForm.value.allowedPaths) agentForm.value.allowedPaths = []
      if (!agentForm.value.allowedPaths.includes(selected)) {
        agentForm.value.allowedPaths.push(selected)
      }
    }
  } catch (error) {
    console.error('Failed to open directory dialog:', error)
  }
}

const removeAuthorizedPath = index => {
  agentForm.value.allowedPaths.splice(index, 1)
}

const removeShellPolicyRule = index => {
  agentForm.value.shellPolicy.splice(index, 1)
}

const clearShellPolicyRules = () => {
  ElMessageBox.confirm(
    t('settings.agent.shellPolicyClearConfirm'),
    t('settings.agent.shellPolicyClearTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'warning'
    }
  ).then(() => {
    agentForm.value.shellPolicy = []
  })
}

const importDefaultShellPolicies = () => {
  ElMessageBox.confirm(
    t('settings.agent.shellPolicyImportDefaultConfirm'),
    t('settings.agent.shellPolicyImportDefaultTitle'),
    {
      confirmButtonText: t('common.confirm'),
      cancelButtonText: t('common.cancel'),
      type: 'info'
    }
  ).then(async () => {
    await ensureDefaultShellPoliciesLoaded()
    if (!agentForm.value.shellPolicy) agentForm.value.shellPolicy = []
    // Add default policies if not already present
    defaultShellPolicies.value.forEach(defaultRule => {
      const exists = agentForm.value.shellPolicy.some(
        rule => rule.pattern === defaultRule.pattern && rule.decision === defaultRule.decision
      )
      if (!exists) {
        agentForm.value.shellPolicy.push({ ...defaultRule })
      }
    })
  })
}

const agentRules = {
  name: [{ required: true, message: t('settings.agent.nameRequired') }],
  systemPrompt: [{ required: true, message: t('settings.agent.systemPromptRequired') }],
  parentAgentId: [
    {
      validator: (_rule, value, callback) => {
        if (agentForm.value.role === AGENT_ROLE.CHILD && !value) {
          callback(new Error(t('settings.agent.parentAgentRequired')))
          return
        }
        callback()
      }
    }
  ]
}

const loadAvailableMcpTools = async () => {
  await agentStore.fetchAvailableTools()
}

const loadSystemSkills = async () => {
  try {
    const result = await invokeWrapper('get_system_skills')
    systemSkills.value = Array.isArray(result) ? result : []
  } catch (error) {
    console.error('Failed to load system skills:', error)
    systemSkills.value = []
  }
}

const loadDefaultShellPolicies = async () => {
  await ensureDefaultShellPoliciesLoaded()
}

const setSelectedSkillsFromSource = selectedSkills => {
  if (Array.isArray(selectedSkills)) {
    shouldBackfillSelectedSkills.value = false
    return sortSkillNamesByName(selectedSkills)
  }

  shouldBackfillSelectedSkills.value = true
  return [...defaultSelectedSkillNames.value]
}

const normalizeAgentFormForSave = form => {
  const normalized = JSON.parse(JSON.stringify(form))

  allModelRoles.forEach(role => {
    const key = role.key + 'Model'
    const model = normalized[key]
    if (!model) return
    model.thinking = model.thinkingEnabled
      ? {
          type: 'enabled',
          budgetTokens: budgetFromThinkingLevel(model.thinkingLevel)
        }
      : null
    delete model.thinkingEnabled
    delete model.thinkingLevel
  })

  normalized.availableTools = Array.isArray(normalized.availableTools)
    ? [...new Set(normalized.availableTools)]
    : []
  normalized.autoApprove = Array.isArray(normalized.autoApprove)
    ? [...new Set(normalized.autoApprove)].filter(
        tool => normalized.availableTools.includes(tool) && tool !== 'bash'
      )
    : []
  normalized.selectedSkills = Array.isArray(normalized.selectedSkills)
    ? sortSkillNamesByName(normalized.selectedSkills)
    : []
  normalized.mcpToolExposure = Array.isArray(normalized.mcpToolExposure)
    ? [...new Set(normalized.mcpToolExposure)]
    : []

  if (normalized.role === AGENT_ROLE.CHILD) {
    normalized.planningPrompt = ''
    normalized.imageRecognitionPrompt = ''
    normalized.planModel = defaultAgentModelConfig()
    normalized.visionModel = defaultAgentModelConfig()
    normalized.utilityModel = defaultAgentModelConfig()
    normalized.allowedPaths = []
    normalized.shellPolicy = []
    normalized.availableTools = normalized.availableTools.filter(tool => tool !== 'bash')
    normalized.autoApprove = normalized.autoApprove.filter(tool => tool !== 'bash')
    normalized.skillEnabled = false
    normalized.selectedSkills = []
    normalized.allowShell = false
  } else {
    normalized.parentAgentId = null
    normalized.shellPolicy = Array.isArray(normalized.shellPolicy)
      ? normalized.shellPolicy.filter(rule => rule.pattern && rule.pattern.trim() !== '')
      : []
    normalized.availableTools = normalized.availableTools.filter(
      tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
    )
    normalized.autoApprove = normalized.autoApprove.filter(tool => !CHILD_ONLY_TOOL_IDS.includes(tool))
    normalized.skillEnabled = normalized.skillEnabled !== false

    if (normalized.allowShell) {
      normalized.availableTools = [...new Set([...normalized.availableTools, 'bash'])]
    } else {
      normalized.availableTools = normalized.availableTools.filter(tool => tool !== 'bash')
      normalized.autoApprove = normalized.autoApprove.filter(tool => tool !== 'bash')
    }
  }

  normalized.isSystem = normalized.isSystem === true
  normalized.disabled = normalized.disabled === true
  normalized.finalAudit = false
  delete normalized.allowShell
  return normalized
}

const syncCurrentWorkflowSkillsConfig = async (savedAgentId, finalForm) => {
  const currentWorkflowId = workflowStore.currentWorkflowId
  const currentWorkflowAgentId = workflowStore.currentWorkflow?.agentId
  if (!currentWorkflowId || !savedAgentId || currentWorkflowAgentId !== savedAgentId) {
    return
  }

  await invokeWrapper('update_workflow_skills_config', {
    sessionId: currentWorkflowId,
    skillEnabled: finalForm.skillEnabled !== false,
    selectedSkills: finalForm.selectedSkills || []
  })

  await workflowStore.selectWorkflow(currentWorkflowId)
}

const getModelList = key => {
  const id = agentForm.value[key + 'Model']?.id
  return id ? modelStore.getModelProviderById(id)?.models || [] : []
}

const onModelIdChange = key => {
  agentForm.value[key + 'Model'].model = ''
}

const applyProviderModelOverrides = (key, modelId) => {
  if (!modelId || modelModes[key] !== 'provider') return

  const selected = getModelList(key).find(model => model.id === modelId)
  if (!selected) return

  const currentModel = agentForm.value[key + 'Model']
  if (selected.temperature !== undefined && selected.temperature !== null) {
    currentModel.temperature = selected.temperature
  }
  currentModel.thinking = selected.thinking || null
  currentModel.thinkingEnabled = !!selected.thinking
  currentModel.thinkingLevel = thinkingLevelFromBudget(selected.thinking?.budgetTokens)
  if (selected.contextSize !== undefined && selected.contextSize !== null) {
    currentModel.contextSize = selected.contextSize
  }
  if (selected.maxTokens !== undefined && selected.maxTokens !== null) {
    currentModel.maxTokens = selected.maxTokens
  }
}

const onProviderModelChange = (key, value) => {
  applyProviderModelOverrides(key, value)
}

const supportsThinking = key => {
  if (modelModes[key] !== 'provider') return !!agentForm.value[key + 'Model']?.thinkingEnabled
  const selected = getModelList(key).find(
    model => model.id === agentForm.value[key + 'Model']?.model
  )
  return !!selected?.reasoning || !!agentForm.value[key + 'Model']?.thinkingEnabled
}

const getProxyAliases = groupName => {
  if (!groupName) return []
  const groupData = settingStore.settings.chatCompletionProxy[groupName]
  return groupData ? Object.keys(groupData) : []
}

const onProxyGroupChange = key => {
  proxyAliases[key] = ''
}

const onProxyAliasChange = (key, value) => {
  agentForm.value[key + 'Model'].model = `${proxyGroups[key]}@${value}`
}

const normalizeModelDraft = model => ({
  ...defaultAgentModelConfig(),
  ...(model || {}),
  thinking: model?.thinking || null,
  thinkingEnabled: !!model?.thinking,
  thinkingLevel: thinkingLevelFromBudget(model?.thinking?.budgetTokens)
})

const parseModelField = (field, key) => {
  if (field && field.id === 0 && field.model?.includes('@')) {
    modelModes[key] = 'proxy'
    const [group, ...rest] = field.model.split('@')
    proxyGroups[key] = group
    proxyAliases[key] = rest.join('@')
  } else {
    modelModes[key] = 'provider'
    proxyGroups[key] = ''
    proxyAliases[key] = ''
  }
}

const editAgent = async id => {
  formRef.value?.resetFields()
  activeTab.value = 'basic'
  await Promise.all([ensureDefaultShellPoliciesLoaded(), loadAvailableMcpTools()])

  if (id) {
    try {
      const agentData = await agentStore.getAgent(id)
      if (!agentData) return
      editId.value = id
      agentForm.value = { ...defaultFormData, ...agentData }
      agentForm.value.allowShell = Array.isArray(agentForm.value.availableTools)
        ? agentForm.value.availableTools.includes('bash')
        : false

      // Ensure tool arrays are sorted by name
      if (agentForm.value.availableTools && Array.isArray(agentForm.value.availableTools)) {
        agentForm.value.availableTools = sortToolIdsByName(agentForm.value.availableTools)
      }
      if (agentForm.value.autoApprove && Array.isArray(agentForm.value.autoApprove)) {
        agentForm.value.autoApprove = sortToolIdsByName(agentForm.value.autoApprove)
      }

      // Unpack unified 'models' field if it exists
      if (agentData.models) {
        try {
          const modelsObj =
            typeof agentData.models === 'string' ? JSON.parse(agentData.models) : agentData.models
          allModelRoles.forEach(role => {
            if (modelsObj[role.key]) {
              agentForm.value[role.key + 'Model'] = normalizeModelDraft(modelsObj[role.key])
              if (agentForm.value[role.key + 'Model'].temperature === undefined) {
                agentForm.value[role.key + 'Model'].temperature = -0.1
              }
            }
          })
        } catch (e) {
          console.error(e)
        }
      }

      // Unpack 'shellPolicy' JSON field if it exists
      if (agentData.shellPolicy) {
        try {
          // Handle both stringified JSON and already parsed array
          if (typeof agentData.shellPolicy === 'string' && agentData.shellPolicy.trim()) {
            const policyObj = JSON.parse(agentData.shellPolicy)
            if (Array.isArray(policyObj)) {
              agentForm.value.shellPolicy = policyObj
            }
          } else if (Array.isArray(agentData.shellPolicy)) {
            // Already an array, use directly
            agentForm.value.shellPolicy = agentData.shellPolicy
          }
        } catch (e) {
          console.error('Failed to parse shellPolicy JSON:', e)
          // Fallback to default policies
          agentForm.value.shellPolicy = cloneDefaultShellPolicies()
        }
      } else {
        // No shell policy, use defaults
        agentForm.value.shellPolicy = cloneDefaultShellPolicies()
      }

      agentForm.value.skillEnabled =
        agentData.skillEnabled !== undefined
          ? Boolean(agentData.skillEnabled)
          : (agentForm.value.role || AGENT_ROLE.PRIMARY) !== AGENT_ROLE.CHILD
      agentForm.value.selectedSkills = setSelectedSkillsFromSource(agentData.selectedSkills)

      // Unpack 'allowedPaths' JSON field if it exists
      const rawPaths = agentData.allowed_paths || agentData.allowedPaths
      if (rawPaths) {
        try {
          if (typeof rawPaths === 'string' && rawPaths.trim()) {
            const pathsObj = JSON.parse(rawPaths)
            if (Array.isArray(pathsObj)) {
              agentForm.value.allowedPaths = pathsObj
            }
          } else if (Array.isArray(rawPaths)) {
            agentForm.value.allowedPaths = rawPaths
          }
        } catch (e) {
          console.error('Failed to parse allowedPaths JSON:', e)
          agentForm.value.allowedPaths = []
        }
      } else {
        agentForm.value.allowedPaths = []
      }

      if ((agentForm.value.role || AGENT_ROLE.PRIMARY) !== AGENT_ROLE.CHILD) {
        agentForm.value.availableTools = (agentForm.value.availableTools || []).filter(
          tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
        )
        agentForm.value.autoApprove = (agentForm.value.autoApprove || []).filter(
          tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
        )
      }

      allModelRoles.forEach(role => parseModelField(agentForm.value[role.key + 'Model'], role.key))
    } catch (error) {
      showMessage(t('settings.agent.fetchFailed'), 'error')
    }
  } else {
    editId.value = null
    agentForm.value = { ...defaultFormData }
    allModelRoles.forEach(role => (modelModes[role.key] = 'provider'))
    agentForm.value.availableTools = availableTools.value
      .map(tool => tool.id)
      .filter(tool => !CHILD_ONLY_TOOL_IDS.includes(tool))
    agentForm.value.autoApprove = availableTools.value
      .filter(tool => READ_ONLY_TOOLS.includes(tool.id))
      .map(tool => tool.id)
    agentForm.value.shellPolicy = cloneDefaultShellPolicies()
    agentForm.value.allowedPaths = []
    agentForm.value.role = AGENT_ROLE.PRIMARY
    agentForm.value.parentAgentId = null
    agentForm.value.isSystem = false
    agentForm.value.disabled = false
    agentForm.value.allowShell = false
    agentForm.value.skillEnabled = true
    shouldBackfillSelectedSkills.value = true
    agentForm.value.selectedSkills = [...defaultSelectedSkillNames.value]
    allModelRoles.forEach(role => {
      agentForm.value[role.key + 'Model'] = normalizeModelDraft(agentForm.value[role.key + 'Model'])
    })
  }

  skillSearchKeyword.value = ''
  agentDialogVisible.value = true
}

const copyAgent = async id => {
  try {
    await ensureDefaultShellPoliciesLoaded()
    const agentData = await agentStore.getAgent(id)
    if (!agentData) return
    agentForm.value = {
      ...defaultFormData,
      ...agentData,
      id: null,
      isSystem: false,
      disabled: true,
      name: `${agentData.name}-Copy`
    }
    agentForm.value.allowShell = Array.isArray(agentForm.value.availableTools)
      ? agentForm.value.availableTools.includes('bash')
      : false
    editId.value = null
    if (agentData.models) {
      try {
        const modelsObj =
          typeof agentData.models === 'string' ? JSON.parse(agentData.models) : agentData.models
        allModelRoles.forEach(role => {
          if (modelsObj[role.key]) {
            agentForm.value[role.key + 'Model'] = normalizeModelDraft(modelsObj[role.key])
          }
        })
      } catch (e) {
        console.error(e)
      }
    }

    // Unpack 'shellPolicy' JSON field if it exists
    if (agentData.shellPolicy) {
      try {
        // Handle both stringified JSON and already parsed array
        if (typeof agentData.shellPolicy === 'string' && agentData.shellPolicy.trim()) {
          const policyObj = JSON.parse(agentData.shellPolicy)
          if (Array.isArray(policyObj)) {
            agentForm.value.shellPolicy = policyObj
          }
        } else if (Array.isArray(agentData.shellPolicy)) {
          // Already an array, use directly
          agentForm.value.shellPolicy = agentData.shellPolicy
        }
      } catch (e) {
        console.error('Failed to parse shellPolicy JSON during copy:', e)
        // Fallback to default policies
        agentForm.value.shellPolicy = cloneDefaultShellPolicies()
      }
    } else {
      // No shell policy, use defaults
      agentForm.value.shellPolicy = cloneDefaultShellPolicies()
    }

    // Unpack 'allowedPaths' JSON field if it exists
    if (agentData.allowedPaths) {
      try {
        if (typeof agentData.allowedPaths === 'string' && agentData.allowedPaths.trim()) {
          const pathsObj = JSON.parse(agentData.allowedPaths)
          if (Array.isArray(pathsObj)) {
            agentForm.value.allowedPaths = pathsObj
          }
        } else if (Array.isArray(agentData.allowedPaths)) {
          agentForm.value.allowedPaths = agentData.allowedPaths
        }
      } catch (e) {
        console.error('Failed to parse allowedPaths JSON during copy:', e)
        agentForm.value.allowedPaths = []
      }
    } else {
      agentForm.value.allowedPaths = []
    }

    agentForm.value.skillEnabled =
      agentData.skillEnabled !== undefined
        ? Boolean(agentData.skillEnabled)
        : (agentForm.value.role || AGENT_ROLE.PRIMARY) !== AGENT_ROLE.CHILD
    agentForm.value.selectedSkills = setSelectedSkillsFromSource(agentData.selectedSkills)

    if ((agentForm.value.role || AGENT_ROLE.PRIMARY) !== AGENT_ROLE.CHILD) {
      agentForm.value.availableTools = (agentForm.value.availableTools || []).filter(
        tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
      )
      agentForm.value.autoApprove = (agentForm.value.autoApprove || []).filter(
        tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
      )
    }

    if (
      !agentForm.value.parentAgentId &&
      agentForm.value.role === AGENT_ROLE.CHILD &&
      primaryAgentOptions.value.length > 0
    ) {
      agentForm.value.parentAgentId = primaryAgentOptions.value[0].id
    }

    allModelRoles.forEach(role => parseModelField(agentForm.value[role.key + 'Model'], role.key))
    skillSearchKeyword.value = ''
    agentDialogVisible.value = true
  } catch (error) {
    showMessage(t('settings.agent.fetchFailed'), 'error')
  }
}

const updateAgent = () => {
  formRef.value.validate(async valid => {
    if (valid) {
      const draftForm = JSON.parse(JSON.stringify(agentForm.value))

      allModelRoles.forEach(role => {
        if (modelModes[role.key] === 'proxy') {
          draftForm[role.key + 'Model'].id = 0
          draftForm[role.key + 'Model'].model = `${proxyGroups[role.key]}@${proxyAliases[role.key]}`
        }
      })

      const finalForm = normalizeAgentFormForSave(draftForm)

      try {
        await agentStore.saveAgent({ ...finalForm, id: editId.value })
        await syncCurrentWorkflowSkillsConfig(editId.value, finalForm)
        showMessage(
          t(editId.value ? 'settings.agent.updateSuccess' : 'settings.agent.addSuccess'),
          'success'
        )
        agentDialogVisible.value = false
        // Refresh the agents list from the store to update the UI
        await agentStore.fetchAgents()
      } catch (error) {
        showMessage(t('settings.agent.saveFailed'), 'error')
      }
    }
  })
}

const deleteAgent = id => {
  ElMessageBox.confirm(t('settings.agent.deleteConfirm'), t('settings.agent.deleteTitle'), {
    confirmButtonText: t('common.confirm'),
    cancelButtonText: t('common.cancel'),
    type: 'warning'
  }).then(async () => {
    try {
      await agentStore.deleteAgent(id)
      showMessage(t('settings.agent.deleteSuccess'), 'success')
    } catch (error) {
      showMessage(t('settings.agent.deleteFailed'), 'error')
    }
  })
}

const persistGroupedAgentOrder = () => {
  agentStore.updateAgentOrder(flattenGroupedAgents()).catch(() => {
    showMessage(t('settings.agent.reorderFailed'), 'error')
    agentStore.fetchAgents()
  })
}

const reorderListByIndexes = (list, oldIndex, newIndex) => {
  if (!Array.isArray(list) || oldIndex === null || newIndex === null || oldIndex === newIndex) {
    return list
  }

  const nextList = [...list]
  const [movedItem] = nextList.splice(oldIndex, 1)
  if (!movedItem) {
    return list
  }
  nextList.splice(newIndex, 0, movedItem)
  return nextList
}

const onPrimarySortUpdate = event => {
  const { oldIndex, newIndex } = event
  groupedPrimaryAgents.value = reorderListByIndexes(groupedPrimaryAgents.value, oldIndex, newIndex)
}

const onChildSortUpdate = (parentId, event) => {
  const { oldIndex, newIndex } = event
  groupedChildAgents.value = {
    ...groupedChildAgents.value,
    [parentId]: reorderListByIndexes(groupedChildAgents.value[parentId] || [], oldIndex, newIndex)
  }
}

const onPrimaryDragEnd = () => {
  persistGroupedAgentOrder()
}

const onChildDragEnd = () => {
  persistGroupedAgentOrder()
}

const toggleAgentStatus = async agent => {
  const originalDisabled = agent.disabled
  try {
    const updatedAgent = {
      ...agent,
      disabled: !agent.disabled
    }
    await agentStore.saveAgent(updatedAgent)
    agent.disabled = !agent.disabled
    const actionText = agent.disabled ? 'disable' : 'enable'
    showMessage(t(`settings.agent.${actionText}Success`, { name: agent.name }), 'success')
  } catch (e) {
    agent.disabled = originalDisabled
    const actionText = originalDisabled ? 'enable' : 'disable'
    showMessage(
      t(`settings.agent.${actionText}Failed`, { error: e.message || String(e), name: agent.name }),
      'error'
    )
  }
}

const onAgentDialogClose = () => {
  // Reset active tab to basic when dialog closes
  activeTab.value = 'basic'
  skillSearchKeyword.value = ''
  // Clear form validation errors
  formRef.value?.resetFields()
}

onMounted(() => {
  modelStore.updateModelStore()
  proxyGroupStore.getList()
  loadDefaultShellPolicies()
  loadSystemSkills()
  loadAvailableMcpTools()
})

watch(
  agents,
  () => {
    syncGroupedAgentLists()
  },
  { deep: true, immediate: true }
)

watch(
  () => agentForm.value.role,
  role => {
    if (role !== AGENT_ROLE.CHILD) {
      agentForm.value.parentAgentId = null
      agentForm.value.skillEnabled = true
      if (
        !Array.isArray(agentForm.value.selectedSkills) ||
        !agentForm.value.selectedSkills.length
      ) {
        agentForm.value.selectedSkills = [...defaultSelectedSkillNames.value]
      }
      agentForm.value.availableTools = (agentForm.value.availableTools || []).filter(
        tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
      )
      agentForm.value.autoApprove = (agentForm.value.autoApprove || []).filter(
        tool => !CHILD_ONLY_TOOL_IDS.includes(tool)
      )
      return
    }

    agentForm.value.allowedPaths = []
    agentForm.value.shellPolicy = []
    agentForm.value.skillEnabled = false
    agentForm.value.selectedSkills = []
    agentForm.value.imageRecognitionPrompt = ''
    agentForm.value.allowShell = false
    agentForm.value.availableTools = (agentForm.value.availableTools || []).filter(
      tool => tool !== 'bash'
    )
    agentForm.value.autoApprove = (agentForm.value.autoApprove || []).filter(
      tool => tool !== 'bash'
    )

    if (!agentForm.value.parentAgentId && primaryAgentOptions.value.length > 0) {
      agentForm.value.parentAgentId = primaryAgentOptions.value[0].id
    }
  }
)

watch(
  () => agentForm.value.availableTools,
  availableToolIds => {
    const enabledToolIds = new Set(availableToolIds || [])
    agentForm.value.mcpToolExposure = (agentForm.value.mcpToolExposure || []).filter(tool =>
      enabledToolIds.has(tool)
    )
  },
  { deep: true }
)

watch(
  () => agentForm.value.allowShell,
  enabled => {
    if (agentForm.value.role === AGENT_ROLE.CHILD) {
      agentForm.value.allowShell = false
      return
    }

    if (!enabled) {
      agentForm.value.availableTools = (agentForm.value.availableTools || []).filter(
        tool => tool !== 'bash'
      )
      agentForm.value.autoApprove = (agentForm.value.autoApprove || []).filter(
        tool => tool !== 'bash'
      )
    }
  }
)

watch(
  defaultSelectedSkillNames,
  names => {
    if (
      shouldBackfillSelectedSkills.value &&
      names.length > 0 &&
      agentForm.value.role !== AGENT_ROLE.CHILD &&
      (!Array.isArray(agentForm.value.selectedSkills) ||
        agentForm.value.selectedSkills.length === 0)
    ) {
      agentForm.value.selectedSkills = [...names]
      shouldBackfillSelectedSkills.value = false
    }
  },
  { immediate: true }
)
</script>

<style lang="scss">
.agent-group {
  display: flex;
  flex-direction: column;
  border-bottom: 1px solid var(--cs-border-color);

  &:last-child {
    border-bottom: none;
  }
}

.agent-group-list,
.agent-child-list {
  display: flex;
  flex-direction: column;
  gap: 0;
}

.item--child {
  margin-left: var(--cs-space-sm);

  .label--child {
    padding-left: var(--cs-space-sm);
  }
}

.agent-edit-dialog {
  .el-dialog__header {
    display: none;
  }

  .el-tabs__nav-wrap:after {
    background-color: var(--cs-border-color);
  }

  .models-layout {
    padding: 4px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;

    &--single {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .form-tip {
    margin-top: 6px;
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    line-height: 1.5;
  }

  .security-switch-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
  }

  .security-switch-label {
    font-size: 13px;
    font-weight: 500;
    color: var(--cs-text-color-primary);
  }

  .security-switch-tip {
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    line-height: 1.5;
  }

  .skill-search-input {
    margin-bottom: 8px;
  }

  .skill-checklist {
    max-height: 500px;
    overflow-y: auto;
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    background-color: var(--cs-bg-color-light);
    padding: 8px;

    &__group {
      display: flex;
      flex-direction: column;
      gap: 8px;
      width: 100%;
    }

    &__item {
      display: flex;
      flex-direction: column;
      gap: 4px;
      padding: 8px 10px;
      border-radius: var(--cs-border-radius-sm);
      background-color: var(--cs-bg-color);
      cursor: pointer;
    }

    &__name {
      color: var(--cs-text-color-primary);
      font-weight: 600;
      margin-left: 4px;
    }

    &__description {
      font-size: 12px;
      color: var(--cs-text-color-secondary);
      padding-left: 26px;
      /* white-space: nowrap;
      text-overflow: ellipsis; */
      max-height: 100px;
      overflow: hidden;
      line-height: 1.4;
    }
  }

  .model-item-compact {
    padding: 8px;
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    background-color: var(--cs-bg-color-light);

    &--full {
      grid-column: 1 / -1;
    }

    .header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 8px;

      .title {
        font-weight: bold;
        font-size: 13px;
        color: var(--cs-text-color-primary);
      }
    }

    .body {
      display: flex;
      flex-direction: column;

      .selectors-row {
        display: flex;
        gap: 4px;
      }

      .params-row {
        display: flex;
        align-items: center;
        gap: 8px;

        &.compact-params {
          justify-content: space-between;
          padding: 0 4px;

          .param-item {
            flex: 1;
            display: flex;
            align-items: center;
            justify-content: space-between;
            gap: 4px;
          }
        }

        .param-label {
          font-size: 11px;
          color: var(--cs-text-color-secondary);
          white-space: nowrap;
        }
      }
    }
  }

  .danger-option {
    color: var(--el-color-danger) !important;
    font-weight: bold;
  }

  .security-group {
    margin-bottom: var(--cs-space-lg);
    display: block;

    .shell-policy-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: var(--cs-space-sm);

      h3 {
        margin: 0;
        font-size: var(--cs-font-size-md);
        color: var(--cs-text-color-primary);
      }

      .shell-policy-actions {
        display: flex;
        gap: var(--cs-space-sm);
        align-items: center;
      }
    }
  }

  .security-tip {
    font-size: 12px;
    color: var(--cs-text-color-secondary);
    margin-bottom: 12px;
    margin-top: -8px;
    line-height: 1.4;
  }

  .shell-policy-list {
    max-height: 300px;
    overflow-y: auto;
    padding-right: 4px;
    margin-top: var(--cs-space-sm);

    /* Custom scrollbar */
    &::-webkit-scrollbar {
      width: 6px;
    }

    &::-webkit-scrollbar-track {
      background: var(--cs-bg-color-light);
      border-radius: 3px;
    }

    &::-webkit-scrollbar-thumb {
      background: var(--cs-border-color);
      border-radius: 3px;

      &:hover {
        background: var(--cs-text-color-secondary);
      }
    }

    .shell-policy-item {
      display: flex;
      gap: var(--cs-space-sm);
      margin-bottom: var(--cs-space-sm);
      align-items: center;
    }
  }
}
</style>
