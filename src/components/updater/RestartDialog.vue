<template>
  <el-dialog
    v-model="visible"
    :title="t('update.readyTitle')"
    width="400px"
    :close-on-click-modal="false"
    :show-close="false">
    <div class="restart-content">
      <p>{{ t('update.readyMessage') }}</p>
      <p v-if="autoRestartSeconds > 0" class="countdown">
        {{ t('update.autoRestartIn', { seconds: autoRestartSeconds }) }}
      </p>
    </div>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="handleLater">{{ t('update.restartLater') }}</el-button>
        <el-button type="primary" @click="handleRestart">
          {{ t('update.restartNow') }}
        </el-button>
      </span>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed, onMounted, onBeforeUnmount, defineEmits, watch } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['update:modelValue', 'restart', 'later'])
const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: value => emit('update:modelValue', value),
})

const COUNTDOWN_SECONDS = 30
const autoRestartSeconds = ref(COUNTDOWN_SECONDS)
let timer = null

onMounted(() => {
  if (props.modelValue) {
    startCountdown()
  }
})

onBeforeUnmount(() => {
  stopCountdown()
})

const startCountdown = () => {
  autoRestartSeconds.value = COUNTDOWN_SECONDS
  timer = setInterval(() => {
    autoRestartSeconds.value--
    if (autoRestartSeconds.value <= 0) {
      handleRestart()
    }
  }, 1000)
}

const stopCountdown = () => {
  if (timer) {
    clearInterval(timer)
    timer = null
  }
}

const handleRestart = () => {
  stopCountdown()
  emit('restart')
}

const handleLater = () => {
  stopCountdown()
  emit('later')
}

watch(
  () => props.modelValue,
  newValue => {
    if (newValue) {
      startCountdown()
    } else {
      stopCountdown()
    }
  }
)
</script>

<style lang="scss" scoped>
.restart-content {
  padding: 20px;
  text-align: center;

  p {
    margin: 0;
    line-height: 1.6;

    &.countdown {
      margin-top: 12px;
      color: var(--el-text-color-secondary);
      font-size: 14px;
    }
  }
}

.dialog-footer {
  display: flex;
  justify-content: center;
  gap: 12px;
}
</style>
