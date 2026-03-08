<template>
  <el-dialog
    v-model="visible"
    :title="title"
    width="500px"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :show-close="false">
    <div class="approval-content">
      <div class="action-info">
        <span class="label">{{ $t('workflow.approval.action') }}:</span>
        <el-tag type="warning">{{ action }}</el-tag>
      </div>
      <div class="details-box">
        <div class="details-header">{{ $t('workflow.approval.details') }}</div>
        <pre class="details-text">{{ details }}</pre>
      </div>
      <p class="warning-text">{{ $t('workflow.approval.warning') }}</p>
    </div>
    <template #footer>
      <div class="dialog-footer">
        <el-button @click="onReject" :loading="loading" round>{{ $t('common.reject') }}</el-button>
        <el-button type="primary" @click="onApprove" :loading="loading" round>{{
          $t('common.approve')
        }}</el-button>
        <el-button type="success" @click="onApproveAll" :loading="loading" round>{{
          $t('common.approveAll')
        }}</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  modelValue: Boolean,
  action: String,
  details: String,
  loading: Boolean
})

const emit = defineEmits(['update:modelValue', 'approve', 'approveAll', 'reject'])

const { t } = useI18n()

const visible = computed({
  get: () => props.modelValue,
  set: val => emit('update:modelValue', val)
})

const title = computed(() => t('workflow.approval.title'))

const onApprove = () => {
  emit('approve')
}

const onApproveAll = () => {
  emit('approveAll')
}

const onReject = () => {
  emit('reject')
}
</script>

<style scoped lang="scss">
.approval-content {
  .action-info {
    margin-bottom: var(--cs-space-md);
    display: flex;
    align-items: center;
    gap: var(--cs-space-sm);

    .label {
      font-weight: bold;
      color: var(--cs-text-color-primary);
    }
  }

  .details-box {
    background-color: var(--cs-bg-color-dark);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    padding: var(--cs-space-sm);
    margin-bottom: var(--cs-space-md);

    .details-header {
      font-size: var(--cs-font-size-xs);
      color: var(--cs-text-color-secondary);
      margin-bottom: var(--cs-space-xs);
      text-transform: uppercase;
    }

    .details-text {
      margin: 0;
      white-space: pre-wrap;
      word-break: break-all;
      font-family: var(--cs-font-family-mono);
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-primary);
      max-height: 200px;
      overflow-y: auto;
    }
  }

  .warning-text {
    color: var(--el-color-danger);
    font-size: var(--cs-font-size-sm);
    font-style: italic;
  }
}
</style>
