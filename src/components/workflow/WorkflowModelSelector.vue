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
              <el-select v-model="currentModel.model" size="small" filterable :disabled="!currentModel.id" style="flex: 1">
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
  contextSize: 128000,
  maxTokens: 0
})

const agentModels = reactive({
  plan: defaultModelConfig(),
  act: defaultModelConfig()
})

const modelModes = reactive({ plan: 'provider', act: 'provider' })
const proxyGroups = reactive({ plan: '', act: '' })
const proxyAliases = reactive({ plan: '', act: '' })

const currentModel = computed(() => agentModels[activeTab.value])

const getModelList = () => {
  const id = currentModel.value.id
  return id ? modelStore.getModelProviderById(id)?.models || [] : []
}

const onModelIdChange = () => {
  currentModel.value.model = ''
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
  if (modelModes.plan === 'proxy') result.plan.id = 0
  if (modelModes.act === 'proxy') result.act.id = 0
  
  emit('save', result)
  visible.value = false
}

const initFromStore = () => {
  // Reset
  agentModels.plan = defaultModelConfig()
  agentModels.act = defaultModelConfig()

  // 1. Get reference agent: 
  // - Direct prop agent
  // - Or last workflow's agent
  let refAgent = props.agent
  if (!refAgent && workflowStore.workflows.length > 0) {
    const lastWf = workflowStore.workflows[0]
    refAgent = agentStore.agents.find(a => a.id === lastWf.agentId)
  }

  // 2. Parse models from agent
  if (refAgent && refAgent.models) {
    try {
      const modelsObj = typeof refAgent.models === 'string' ? JSON.parse(refAgent.models) : refAgent.models
      if (modelsObj.plan) agentModels.plan = { ...defaultModelConfig(), ...modelsObj.plan }
      if (modelsObj.act) agentModels.act = { ...defaultModelConfig(), ...modelsObj.act }
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
  }

  // Parse UI states
  parseModelField(agentModels.plan, 'plan')
  parseModelField(agentModels.act, 'act')
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
