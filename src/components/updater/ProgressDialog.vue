<template>
  <el-dialog
    v-model="visible"
    :title="t('update.downloading')"
    width="400px"
    :close-on-click-modal="false"
    :show-close="false">
    <div class="progress-content">
      <el-progress :percentage="progress" :status="status" :format="progressFormat" />
      <p class="status-text" v-if="error">{{ error }}</p>
    </div>
    <template #footer>
      <span class="dialog-footer">
        <el-button @click="handleCancel" :disabled="!canCancel">
          {{ t('update.cancelDownload') }}
        </el-button>
      </span>
    </template>
  </el-dialog>
</template>

<script setup>
import { computed, defineEmits } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false,
  },
  progress: {
    type: Number,
    default: 0,
  },
  error: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['update:modelValue', 'cancel'])
const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: value => emit('update:modelValue', value),
})

const status = computed(() => {
  if (props.error) return 'exception'
  if (props.progress >= 100) return 'success'
  return ''
})

const canCancel = computed(() => {
  return props.progress < 100 && !props.error
})

const progressFormat = percentage => {
  if (props.error) return t('update.downloadFailed')
  if (percentage >= 100) return t('update.downloadCompleted')
  return `${percentage}%`
}

const handleCancel = () => {
  emit('cancel')
}
</script>

<style lang="scss" scoped>
.progress-content {
  padding: 20px;

  .status-text {
    margin: 16px 0 0;
    color: var(--el-color-danger);
    font-size: 14px;
    text-align: center;
  }
}

.dialog-footer {
  display: flex;
  justify-content: center;
}
</style>
