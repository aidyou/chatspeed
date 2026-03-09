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
        <div v-if="isEditAction" class="diff-view">
          <markdown :content="diffMarkdown" />
        </div>
        <pre v-else class="details-text">{{ details }}</pre>
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
import Markdown from '@/components/chat/Markdown.vue'

const props = defineProps({
  modelValue: Boolean,
  action: String,
  details: String,
  loading: Boolean
})

const emit = defineEmits(['update:modelValue', 'approve', 'approveAll', 'reject'])

const { t } = useI18n()

const isEditAction = computed(() => props.action === 'edit_file')

const diffMarkdown = computed(() => {
  if (!isEditAction.value) return ''
  try {
    const data = JSON.parse(props.details)
    const oldStr = data.old_string || ''
    const newStr = data.new_string || ''
    const filePath = data.file_path || data.path || 'unknown file'
    
    return `### ${filePath}\n\n\`\`\`diff\n--- Original\n+++ Modified\n${generateSimpleDiff(oldStr, newStr)}\n\`\`\``
  } catch (e) {
    return props.details
  }
})

// Simple line-based diff generator since we don't have a heavy diff library
const generateSimpleDiff = (oldStr, newStr) => {
  if (oldStr === newStr) return ' (No visible changes)'
  
  const oldLines = oldStr.split('\n')
  const newLines = newStr.split('\n')
  let diff = ''
  
  // For now, showing a simple "Removed/Added" block is much clearer than raw JSON
  oldLines.forEach(line => {
    if (line.trim()) diff += `- ${line}\n`
  })
  newLines.forEach(line => {
    if (line.trim()) diff += `+ ${line}\n`
  })
  
  return diff || ' (No visible changes)'
}

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

    .diff-view {
      max-height: 350px;
      overflow-y: auto;
      background-color: var(--cs-bg-color);
      border-radius: var(--cs-border-radius-sm);
      padding: 4px;

      :deep(.markdown-body) {
        font-size: 12px;
        background: transparent !important;
        
        pre {
          background: var(--cs-bg-color-dark) !important;
          border: none;
        }

        h3 {
          font-size: 13px;
          margin-top: 0;
          color: var(--el-color-primary);
        }
      }
    }
  }

  .warning-text {
    color: var(--el-color-danger);
    font-size: var(--cs-font-size-sm);
    font-style: italic;
  }
}
</style>
