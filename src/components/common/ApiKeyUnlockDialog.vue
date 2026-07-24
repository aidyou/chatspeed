<template>
  <el-dialog
    v-model="visible"
    width="480px"
    :title="t('settings.general.apiKeyProtection.unlockTitle')"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :append-to-body="true">
    <div class="api-key-unlock-content">
      <cs name="warning" size="32px" color="var(--el-color-warning)" />
      <div>
        <p>{{ unlockMessage }}</p>
        <small v-if="status?.keyFile" :title="status.keyFile">{{ status.keyFile }}</small>
      </div>
    </div>
    <template #footer>
      <el-button
        type="primary"
        :loading="selecting"
        :disabled="status?.state === 'unsupported'"
        @click="selectKeyFile">
        <cs name="ext-folder-open" />
        {{ t('settings.general.apiKeyProtection.selectFile') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { open } from '@tauri-apps/plugin-dialog'
import { listen } from '@tauri-apps/api/event'

import { invokeWrapper } from '@/libs/tauri'
import { showMessage } from '@/libs/util'
import { sendSyncState } from '@/libs/sync'
import { useModelStore } from '@/stores/model'

const props = defineProps({
  active: {
    type: Boolean,
    default: false
  },
  windowLabel: {
    type: String,
    required: true
  }
})

const { t } = useI18n()
const modelStore = useModelStore()
const visible = ref(false)
const selecting = ref(false)
const status = ref(null)
let unlistenSyncState = null

const unlockMessage = computed(() => {
  const reason = status.value?.reason || 'key_file_not_configured'
  return t(`settings.general.apiKeyProtection.reasons.${reason}`)
})

const refreshStatus = async () => {
  status.value = await invokeWrapper('get_api_key_encryption_status')
  visible.value = props.active && ['locked', 'unsupported'].includes(status.value?.state)
}

const selectKeyFile = async () => {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: t('settings.general.apiKeyProtection.fileFilterName'), extensions: ['csk', 'json'] }]
  })
  if (!path) return

  selecting.value = true
  try {
    status.value = await invokeWrapper('activate_api_key_file', { path })
    visible.value = false
    modelStore.updateModelStore()
    sendSyncState('model', props.windowLabel)
    showMessage(t('settings.general.apiKeyProtection.unlockSuccess'), 'success')
  } catch (error) {
    console.error('Failed to activate API key file:', error)
    showMessage(t('settings.general.apiKeyProtection.operationFailed'), 'error')
  } finally {
    selecting.value = false
  }
}

onMounted(async () => {
  if (!props.active) return

  try {
    unlistenSyncState = await listen('cs://sync-state', event => {
      if (event?.payload?.type !== 'model') return
      refreshStatus()
        .then(() => {
          if (!['locked', 'unsupported'].includes(status.value?.state)) {
            modelStore.updateModelStore()
          }
        })
        .catch(error => {
          console.error('Failed to refresh API key encryption status:', error)
        })
    })
    await refreshStatus()
  } catch (error) {
    console.error('Failed to inspect API key encryption status:', error)
    showMessage(t('settings.general.apiKeyProtection.statusFailed'), 'error')
  }
})

onBeforeUnmount(() => {
  unlistenSyncState?.()
})
</script>

<style scoped lang="scss">
.api-key-unlock-content {
  display: flex;
  align-items: flex-start;
  gap: var(--cs-space-md);

  p {
    margin: 0;
    line-height: 1.6;
    color: var(--cs-text-color-primary);
  }

  small {
    display: block;
    max-width: 380px;
    margin-top: var(--cs-space-xs);
    overflow: hidden;
    color: var(--cs-text-color-secondary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}
</style>
