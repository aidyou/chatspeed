<template>
  <section class="decision-settings">
    <div class="card">
      <div class="title">{{ $t('settings.sandbox.decisionTitle') }}</div>
      <div class="list">
        <div class="item">
          <div class="label">
            <div class="label-text">
              {{ $t('settings.sandbox.decisionEnabled') }}
              <small class="tooltip">{{ $t('settings.sandbox.decisionEnabledHint') }}</small>
            </div>
          </div>
          <div class="value">
            <el-switch v-model="draft.enabled" @change="saveConfig" />
          </div>
        </div>
        <div class="item">
          <div class="label">{{ $t('settings.sandbox.decisionProvider') }}</div>
          <div class="value decision-selectors">
            <el-select v-model="draft.providerId" clearable filterable :disabled="!draft.enabled"
              :placeholder="$t('settings.sandbox.decisionProviderPlaceholder')" @change="onProviderChange">
              <el-option v-for="provider in decisionProviders" :key="provider.id" :label="provider.name"
                :value="provider.id" />
            </el-select>
            <el-select v-model="draft.model" clearable filterable :disabled="!draft.enabled || !draft.providerId"
              :placeholder="$t('settings.sandbox.decisionModelPlaceholder')" @change="saveConfig">
              <el-option v-for="model in decisionModels" :key="decisionModelId(model)"
                :label="model.name || decisionModelId(model)" :value="decisionModelId(model)" />
            </el-select>
          </div>
        </div>
        <div v-if="draft.enabled && !isValidSelection" class="form-tip">
          {{ $t('settings.sandbox.decisionModelRequired') }}
        </div>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed, reactive, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'

import { showMessage } from '@/libs/util'
import { useModelStore } from '@/stores/model'
import { useSettingStore } from '@/stores/setting'

const { t } = useI18n()
const modelStore = useModelStore()
const settingStore = useSettingStore()
const { settings } = storeToRefs(settingStore)

const draft = reactive({ enabled: false, providerId: null, model: '' })
const decisionProviders = computed(() =>
  modelStore.providers.filter(provider => !provider.disabled && provider.apiProtocol === 'decision')
)
const selectedProvider = computed(() =>
  decisionProviders.value.find(provider => provider.id === draft.providerId) || null
)
const decisionModels = computed(() => selectedProvider.value?.models || [])
const decisionModelId = model => String(model?.id || model?.name || '').trim()
const isValidSelection = computed(() =>
  !!selectedProvider.value && decisionModels.value.some(model => decisionModelId(model) === draft.model)
)

const syncDraft = config => {
  const value = config && typeof config === 'object' ? config : {}
  draft.enabled = value.enabled === true
  draft.providerId = Number.isFinite(Number(value.providerId)) ? Number(value.providerId) : null
  draft.model = String(value.model || '').trim()
}

watch(() => settings.value.decisionConfig, syncDraft, { immediate: true, deep: true })
watch(decisionProviders, providers => {
  if (!providers.some(provider => provider.id === draft.providerId)) {
    draft.providerId = null
    draft.model = ''
  }
}, { immediate: true })

const saveConfig = async () => {
  try {
    await settingStore.setSetting('decisionConfig', {
      enabled: draft.enabled,
      providerId: draft.providerId,
      model: draft.model
    })
  } catch (error) {
    showMessage(error?.message || t('settings.sandbox.decisionSaveFailed'), 'error')
  }
}

const onProviderChange = () => {
  draft.model = ''
  void saveConfig()
}
</script>

<style scoped lang="scss">
.decision-settings {
  max-width: 1080px;
  margin: 0 auto;

  .decision-selectors {
    display: flex;
    gap: var(--cs-space-sm);
    width: min(100%, 560px);
  }

  .decision-selectors .el-select {
    flex: 1;
  }

  .form-tip {
    padding: var(--cs-space-sm) var(--cs-space);
    color: var(--el-color-warning);
  }
}
</style>
