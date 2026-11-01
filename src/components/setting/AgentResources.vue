<template>
  <div class="card agent-resource-settings">
    <div class="title">{{ $t('settings.agent.resourceTitle') }}</div>
    <div class="list">
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceEnabled') }}</div>
        <div class="value"><el-switch v-model="resourceLimits.enabled" :disabled="resourceSaving" /></div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceScope') }}</div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceTotalMemory') }}</div>
        <div class="value">
          <el-input-number v-model="resourceLimits.totalMemoryMb" :min="128" :max="1048576" :step="128"
            :precision="0" :disabled="!resourceLimits.enabled || resourceSaving" controls-position="right" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceConcurrentTasks') }}</div>
        <div class="value">
          <el-input-number v-model="resourceLimits.maxConcurrentTasks" :min="1" :max="64" :precision="0"
            :disabled="!resourceLimits.enabled || resourceSaving" controls-position="right" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceTaskMemory') }}</div>
        <div class="value">
          <el-input-number v-model="resourceLimits.taskMemoryMb" :min="128" :max="resourceLimits.totalMemoryMb"
            :step="128" :precision="0" :disabled="!resourceLimits.enabled || resourceSaving" controls-position="right" />
        </div>
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceTaskProcesses') }}</div>
        <div class="value">
          <el-input-number v-model="resourceLimits.taskMaxProcesses" :min="1" :max="4096" :precision="0"
            :disabled="!resourceLimits.enabled || resourceSaving" controls-position="right" />
        </div>
      </div>
      <div class="item">
        <el-alert :title="$t('settings.agent.resourceProtectionHint')" type="warning" :closable="false" show-icon />
      </div>
      <div class="item">
        <div class="label">{{ $t('settings.agent.resourceResetHint') }}</div>
        <div class="value">
          <el-button type="primary" :loading="resourceSaving" @click="saveResourceLimits">
            {{ $t('settings.agent.resourceSave') }}
          </el-button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSettingStore } from '@/stores/setting'
import { showMessage } from '@/libs/util'
import { formatUserError } from '@/libs/error'
import { DEFAULT_AGENT_RESOURCE_LIMITS, validateAgentResourceLimits } from '@/libs/agentResourceLimits'

const { t } = useI18n()
const settingStore = useSettingStore()
const resourceLimits = ref({ ...DEFAULT_AGENT_RESOURCE_LIMITS })
const resourceSaving = ref(false)

watch(
  () => settingStore.settings.agentResourceLimits,
  value => { resourceLimits.value = { ...DEFAULT_AGENT_RESOURCE_LIMITS, ...value } },
  { immediate: true, deep: true }
)

const saveResourceLimits = async () => {
  if (!validateAgentResourceLimits(resourceLimits.value)) {
    showMessage(t('settings.agent.resourceInvalid'), 'error')
    return
  }
  resourceSaving.value = true
  try {
    await settingStore.setSetting('agentResourceLimits', { ...resourceLimits.value })
    showMessage(t('common.saveSuccess'), 'success')
  } catch (error) {
    showMessage(formatUserError(error, t('common.saveFailed')), 'error')
  } finally {
    resourceSaving.value = false
  }
}
</script>

<style scoped lang="scss">
.agent-resource-settings .el-input-number {
  width: 160px;
}
</style>
