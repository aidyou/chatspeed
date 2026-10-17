<template>
  <el-dialog
    v-model="active"
    class="quit-confirm-dialog"
    width="480px"
    :title="t('quitConfirm.title')"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="!busy"
    :append-to-body="true"
    align-center
    @closed="handleClosed">
    <div class="quit-confirm-content">
      <p class="quit-confirm-impact">{{ t('quitConfirm.impact') }}</p>
      <el-checkbox v-model="stopRuntime" :disabled="busy" class="quit-confirm-checkbox">
        {{ t('quitConfirm.stopRuntime') }}
      </el-checkbox>
      <p class="quit-confirm-hint">{{ t('quitConfirm.stopRuntimeHint') }}</p>
      <div v-if="busy && !failed" class="quit-confirm-status">
        <cs name="loading" class="quit-confirm-spinner" />
        <span>{{ t('quitConfirm.stopping') }}</span>
      </div>
      <el-alert
        v-else-if="failed"
        class="quit-confirm-error"
        type="error"
        :closable="false"
        show-icon
        :title="t(`quitConfirm.errors.${error}`)" />
    </div>
    <template #footer>
      <el-button :disabled="busy" @click="handleCancel">{{ t('quitConfirm.cancel') }}</el-button>
      <template v-if="failed">
        <el-button type="primary" :loading="submitting" :disabled="submitting" @click="handleRetry">
          {{ t('quitConfirm.retry') }}
        </el-button>
        <el-button :disabled="submitting" @click="handleExitDesktopOnly">
          {{ t('quitConfirm.exitDesktopOnly') }}
        </el-button>
      </template>
      <el-button v-else type="primary" :loading="submitting" :disabled="busy" @click="handleExit">
        {{ t('quitConfirm.exit') }}
      </el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { useI18n } from 'vue-i18n'

import {
  QUIT_PHASE_CONFIRMING,
  useQuitConfirmation
} from '@/composables/useQuitConfirmation'

const { t } = useI18n()
const { active, stopRuntime, phase, error, submitting, busy, failed, submit, cancel } =
  useQuitConfirmation()

const handleExit = () => {
  submit()
}

// A retry always carries the stop choice: the failure came from a stop attempt,
// so the checkbox is re-armed rather than silently downgraded to a desktop-only exit.
const handleRetry = () => {
  stopRuntime.value = true
  submit()
}

// The user's escape hatch after a failed stop: leave the runtime untouched and
// quit only this desktop.
const handleExitDesktopOnly = () => {
  stopRuntime.value = false
  submit()
}

const handleCancel = () => {
  cancel()
}

// ESC closes the dialog through the model; the pending prompt still has to be
// cancelled on the coordinator so the next tray click starts a fresh prompt.
const handleClosed = () => {
  if (phase.value === QUIT_PHASE_CONFIRMING) {
    cancel()
  }
}
</script>

<style scoped lang="scss">
.quit-confirm-content {
  display: flex;
  flex-direction: column;
  gap: var(--cs-space-sm);

  .quit-confirm-impact {
    margin: 0;
    line-height: 1.6;
    color: var(--cs-text-color-primary);
  }

  .quit-confirm-checkbox {
    margin-top: var(--cs-space-xs);
  }

  .quit-confirm-hint {
    margin: 0;
    line-height: 1.5;
    font-size: var(--cs-font-size-sm);
    color: var(--cs-text-color-secondary);
  }

  .quit-confirm-status {
    display: flex;
    align-items: center;
    gap: var(--cs-space-xs);
    margin-top: var(--cs-space-xs);
    color: var(--cs-text-color-secondary);
  }

  .quit-confirm-spinner {
    animation: quit-confirm-spin 1s linear infinite;
  }

  .quit-confirm-error {
    margin-top: var(--cs-space-xs);
  }
}

@keyframes quit-confirm-spin {
  from {
    transform: rotate(0deg);
  }

  to {
    transform: rotate(360deg);
  }
}
</style>