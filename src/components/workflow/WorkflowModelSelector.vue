<template>
  <el-dialog
    v-model="visible"
    :title="$t('settings.model.modelConfig') || 'Model Configuration'"
    width="480px"
    custom-class="model-selector-dialog"
    :before-close="handleClose"
  >
    <div class="model-selector-content">
      <el-tabs v-model="activeTab" class="model-tabs">
        <el-tab-pane label="PLAN" name="plan"></el-tab-pane>
        <el-tab-pane label="ACT" name="act"></el-tab-pane>
        <el-tab-pane label="UTILITY" name="utility"></el-tab-pane>
      </el-tabs>

      <div class="model-item-compact">
        <div class="header">
          <span class="title">{{ activeTab.toUpperCase() }} {{ $t('settings.agent.models') }}</span>
          <el-radio-group v-model="modelModes[activeTab]" size="small">
            <el-radio-button value="provider">{{ $t('settings.agent.modeProvider') }}</el-radio-button>
            <el-radio-button value="proxy">{{ $t('settings.agent.modeProxy') }}</el-radio-button>
          </el-radio-group>
        </div>
        
        <div class="body">
          <div class="selectors-row">
            <template v-if="modelModes[activeTab] === 'provider'">
              <el-select v-model="currentModel.id" size="small" filterable @change="onModelIdChange" style="width: 120px">
                <el-option v-for="provider in modelStore.getAvailableProviders" :key="provider.id"
                  :label="provider.name" :value="provider.id" />
              </el-select>
              <el-select v-model="currentModel.model" size="small" filterable :disabled="!currentModel.id" @change="onProviderModelChange" style="flex: 1">
                <el-option v-for="model in getModelList()" :key="model.id" :label="model.name || model.id" :value="model.id" />
              </el-select>
            </template>
            <template v-else>
              <el-select v-model="proxyGroups[activeTab]" size="small" filterable @change="onProxyGroupChange" style="width: 120px">
                <el-option v-for="group in proxyGroupStore.list" :key="group.name" :label="group.name" :value="group.name" />
              </el-select>
              <el-select v-model="proxyAliases[activeTab]" size="small" filterable :disabled="!proxyGroups[activeTab]"
                @change="onProxyAliasChange" style="flex: 1">
                <el-option v-for="alias in getProxyAliases(proxyGroups[activeTab])" :key="alias" :label="alias" :value="alias" />
              </el-select>
            </template>
          </div>

          <div class="params-row" style="margin-top: 12px;">
            <span class="param-label">{{ $t('settings.agent.temperature') }}</span>
            <el-slider v-model="currentModel.temperature" :min="-0.1" :max="2" :step="0.1" size="small" style="flex: 1; margin: 0 12px;" />
            <span class="param-value" style="font-size: 11px; min-width: 24px; text-align: right;">
              {{ currentModel.temperature < 0 ? 'Off' : currentModel.temperature.toFixed(1) }}
            </span>
          </div>

          <div class="params-row compact-params" style="margin-top: 8px; display: flex; gap: 10px;">
            <div class="param-item" style="flex: 1;">
              <span class="param-label" style="display: block; margin-bottom: 4px;">{{ $t('settings.model.contextSize') }}</span>
              <el-input-number v-model="currentModel.contextSize" :min="1024" :max="2000000" :step="1024"
                size="small" controls-position="right" style="width: 100%" />
            </div>
            <div class="param-item" style="flex: 1;">
              <span class="param-label" style="display: block; margin-bottom: 4px;">{{ $t('settings.model.maxTokens') }}</span>
              <el-input-number v-model="currentModel.maxTokens" :min="0" :max="128000" :step="1024"
                size="small" controls-position="right" style="width: 100%" />
            </div>
          </div>

          <div v-if="supportsThinking(activeTab)" class="params-row compact-params" style="margin-top: 8px; display: flex; gap: 10px;">
            <div class="param-item" style="flex: 1;">
              <span class="param-label" style="display: block; margin-bottom: 4px;">{{ $t('settings.model.reasoning') }}</span>
              <el-switch v-model="currentModel.thinkingEnabled" size="small" />
            </div>
            <div class="param-item" v-if="currentModel.thinkingEnabled" style="flex: 1;">
              <span class="param-label" style="display: block; margin-bottom: 4px;">{{ $t('settings.model.thinkingLevel') }}</span>
              <el-select v-model="currentModel.thinkingLevel" size="small" style="width: 100%">
                <el-option
                  v-for="option in workflowThinkingLevelOptions"
                  :key="option.value"
                  :label="$t(option.label)"
                  :value="option.value" />
              </el-select>
            </div>
          </div>
        </div>
      </div>
    </div>
    <template #footer>
      <div class="dialog-footer">
        <el-button @click="visible = false" size="small">Cancel</el-button>
        <el-button type="primary" @click="handleSave" size="small">Save</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, watch, onMounted, reactive } from 'vue'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'
import { useWorkflowStore } from '@/stores/workflow'
import { useProxyGroupStore } from '@/stores/proxy_group'
import { useAgentStore } from '@/stores/agent'

const props = defineProps({
  modelValue: Boolean,
  initialTab: {
    type: String,
    default: 'act'
  },
  agent: {
    type: Object,
    default: null
  }
})

const emit = defineEmits(['update:modelValue', 'save'])

const modelStore = useModelStore()
const settingStore = useSettingStore()
const workflowStore = useWorkflowStore()
const proxyGroupStore = useProxyGroupStore()
const agentStore = useAgentStore()

const visible = computed({
  get: () => props.modelValue,
  set: (val) => emit('update:modelValue', val)
})

const activeTab = ref(props.initialTab)

const defaultModelConfig = () => ({
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
const thinkingLevelFromBudget = (budget) => {
  const normalized = Number(budget) || 0
  if (normalized > 2048) return 'high'
  if (normalized > 1024) return 'medium'
  return 'low'
}
const budgetFromThinkingLevel = (level) => THINKING_LEVEL_TO_BUDGET[level] || THINKING_LEVEL_TO_BUDGET.low
const workflowThinkingLevelOptions = [
  { value: 'low', label: 'settings.model.reasoningLow' },
  { value: 'medium', label: 'settings.model.reasoningMedium' },
  { value: 'high', label: 'settings.model.reasoningHigh' }
]

const agentModels = reactive({
  plan: defaultModelConfig(),
  act: defaultModelConfig(),
  utility: defaultModelConfig()
})

const modelModes = reactive({ plan: 'provider', act: 'provider', utility: 'provider' })
const proxyGroups = reactive({ plan: '', act: '', utility: '' })
const proxyAliases = reactive({ plan: '', act: '', utility: '' })

const currentModel = computed(() => agentModels[activeTab.value])

const getModelList = () => {
  const id = currentModel.value.id
  return id ? modelStore.getModelProviderById(id)?.models || [] : []
}

const onModelIdChange = () => {
  currentModel.value.model = ''
}

const normalizeModelDraft = (model) => ({
  ...defaultModelConfig(),
  ...(model || {}),
  thinking: model?.thinking || null,
  thinkingEnabled: !!model?.thinking,
  thinkingLevel: thinkingLevelFromBudget(model?.thinking?.budgetTokens)
})

const applyProviderModelOverrides = (modelId) => {
  if (!modelId || modelModes[activeTab.value] !== 'provider') return

  const selected = getModelList().find(model => model.id === modelId)
  if (!selected) return

  if (selected.temperature !== undefined && selected.temperature !== null) {
    currentModel.value.temperature = selected.temperature
  }
  currentModel.value.thinking = selected.thinking || null
  currentModel.value.thinkingEnabled = !!selected.thinking
  currentModel.value.thinkingLevel = thinkingLevelFromBudget(selected.thinking?.budgetTokens)
  if (selected.contextSize !== undefined && selected.contextSize !== null) {
    currentModel.value.contextSize = selected.contextSize
  }
  if (selected.maxTokens !== undefined && selected.maxTokens !== null) {
    currentModel.value.maxTokens = selected.maxTokens
  }
}

const onProviderModelChange = (value) => {
  applyProviderModelOverrides(value)
}

const getProxyAliases = (groupName) => {
  if (!groupName) return []
  const groupData = settingStore.settings.chatCompletionProxy[groupName]
  return groupData ? Object.keys(groupData) : []
}

const onProxyGroupChange = () => {
  proxyAliases[activeTab.value] = ''
}

const onProxyAliasChange = (value) => {
  currentModel.value.model = `${proxyGroups[activeTab.value]}@${value}`
}

const supportsThinking = (key) => {
  if (modelModes[key] !== 'provider') return !!agentModels[key]?.thinkingEnabled
  const selected = (agentModels[key]?.id
    ? modelStore.getModelProviderById(agentModels[key].id)?.models || []
    : []
  ).find(model => model.id === agentModels[key]?.model)
  return !!selected?.reasoning || !!agentModels[key]?.thinkingEnabled
}

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

const handleClose = (done) => {
  visible.value = false
  done()
}

const handleSave = () => {
  const result = JSON.parse(JSON.stringify(agentModels))
  for (const key of ['plan', 'act', 'utility']) {
    result[key].thinking = result[key].thinkingEnabled
      ? {
        type: 'enabled',
        budgetTokens: budgetFromThinkingLevel(result[key].thinkingLevel)
      }
      : null
    delete result[key].thinkingEnabled
    delete result[key].thinkingLevel
  }
  if (modelModes.plan === 'proxy') result.plan.id = 0
  if (modelModes.act === 'proxy') result.act.id = 0
  if (modelModes.utility === 'proxy') result.utility.id = 0
  
  emit('save', result)
  visible.value = false
}

const initFromStore = () => {
  // Reset
  agentModels.plan = defaultModelConfig()
  agentModels.act = defaultModelConfig()
  agentModels.utility = defaultModelConfig()

  // 1. Get reference agent:
  // - Direct prop agent
  // - Or current workflow's agentConfig (has higher priority if workflow is active)
  // - Or last workflow's agent
  let refAgent = props.agent

  // 1a. Check current workflow's agentConfig first (higher priority for active workflow)
  const currentWf = workflowStore.currentWorkflow
  if (currentWf?.agentConfig?.models) {
    const wfModels = currentWf.agentConfig.models
    if (wfModels.plan) agentModels.plan = normalizeModelDraft(wfModels.plan)
    if (wfModels.act) agentModels.act = normalizeModelDraft(wfModels.act)
    if (wfModels.utility) agentModels.utility = normalizeModelDraft(wfModels.utility)
  } else if (!refAgent && workflowStore.workflows.length > 0) {
    // 1b. Fallback to last workflow's agent
    const lastWf = workflowStore.workflows[0]
    refAgent = agentStore.agents.find(a => a.id === lastWf.agentId)
  }

  // 2. Parse models from agent (only if workflow didn't have models)
  if (!workflowStore.currentWorkflow?.agentConfig?.models && refAgent && refAgent.models) {
    try {
      const modelsObj = typeof refAgent.models === 'string' ? JSON.parse(refAgent.models) : refAgent.models
      if (modelsObj.plan) agentModels.plan = normalizeModelDraft(modelsObj.plan)
      if (modelsObj.act) agentModels.act = normalizeModelDraft(modelsObj.act)
      if (modelsObj.utility) agentModels.utility = normalizeModelDraft(modelsObj.utility)
    } catch (e) {
      console.error('Failed to parse agent models:', e)
    }
  }

  // 3. Absolute fallback to system default provider if still empty
  const fallbackP = modelStore.getAvailableProviders.find(p => p.isDefault) || modelStore.getAvailableProviders[0]
  if (fallbackP) {
    if (!agentModels.plan.id && !agentModels.plan.model.includes('@')) {
      agentModels.plan.id = fallbackP.id
      agentModels.plan.model = fallbackP.defaultModel
    }
    if (!agentModels.act.id && !agentModels.act.model.includes('@')) {
      agentModels.act.id = fallbackP.id
      agentModels.act.model = fallbackP.defaultModel
    }
    if (!agentModels.utility.id && !agentModels.utility.model.includes('@')) {
      agentModels.utility.id = fallbackP.id
      agentModels.utility.model = fallbackP.defaultModel
    }
  }

  // Parse UI states
  parseModelField(agentModels.plan, 'plan')
  parseModelField(agentModels.act, 'act')
  parseModelField(agentModels.utility, 'utility')
}

onMounted(() => {
  proxyGroupStore.getList()
  initFromStore()
})

watch(visible, (newVal) => {
  if (newVal) initFromStore()
})

watch(() => props.initialTab, (val) => {
  activeTab.value = val
})
</script>

<style lang="scss" scoped>
.model-selector-content {
  padding: 4px;
}

.model-tabs {
  margin-bottom: 16px;
}

.model-item-compact {
  padding: 12px;
  border: 1px solid var(--cs-border-color);
  border-radius: var(--cs-border-radius-md);
  background-color: var(--cs-bg-color-light);

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 12px;

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
      gap: 6px;
    }

    .params-row {
      display: flex;
      align-items: center;

      .param-label {
        font-size: 11px;
        color: var(--cs-text-color-secondary);
        white-space: nowrap;
      }
    }
  }
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
