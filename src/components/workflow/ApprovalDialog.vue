<template>
  <el-dialog v-model="visible" :title="title" :width="dialogWidth" :close-on-click-modal="false"
    :close-on-press-escape="false" :show-close="false" :class="{ 'diff-dialog': isEditAction }"
    :modal-class="isEditAction ? 'diff-dialog-overlay' : ''" custom-class="approval-dialog">
    <div class="approval-content">
      <div class="action-info">
        <span class="label">{{ $t('workflow.approval.action') }}:</span>
        <el-tag type="warning">{{ action }}</el-tag>
        <span v-if="isEditAction && filePath" class="file-path">{{ filePath }}</span>
      </div>
      <div class="details-box">
        <div class="details-header">
          <span>{{ $t('workflow.approval.details') }}</span>
          <span class="warning-text">{{ $t('workflow.approval.warning') }}</span>
        </div>
        <div v-if="isEditAction" class="diff-view">
          <markdown-simple :content="diffMarkdown" />
        </div>
        <pre v-else class="details-text">{{ details }}</pre>
      </div>
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
import MarkdownSimple from '@/components/workflow/MarkdownSimple.vue'
import * as Diff from 'diff'

const props = defineProps({
  modelValue: Boolean,
  action: String,
  details: String,
  loading: Boolean
})

const emit = defineEmits(['update:modelValue', 'approve', 'approveAll', 'reject'])

const { t } = useI18n()

const isEditAction = computed(() => props.action === 'edit_file')

const filePath = computed(() => {
  if (!isEditAction.value) return ''
  try {
    const data = JSON.parse(props.details)
    return data.file_path || data.path || ''
  } catch (e) {
    return ''
  }
})

const diffMarkdown = computed(() => {
  if (!isEditAction.value) return ''
  try {
    const data = JSON.parse(props.details)
    const oldStr = data.old_string || ''
    const newStr = data.new_string || ''

    return `\`\`\`diff\n${generateDiff(oldStr, newStr)}\n\`\`\``
  } catch (e) {
    return props.details
  }
})

// Use diff library to generate proper line-by-line diff
const generateDiff = (oldStr, newStr) => {
  if (oldStr === newStr) return ' (No visible changes)'

  const changes = Diff.diffLines(oldStr, newStr)
  let diff = ''

  changes.forEach(change => {
    const lines = change.value.split('\n')

    // Remove last empty line if exists
    if (lines[lines.length - 1] === '') {
      lines.pop()
    }

    lines.forEach(line => {
      if (change.added) {
        diff += `+ ${line}\n`
      } else if (change.removed) {
        diff += `- ${line}\n`
      } else {
        // Unchanged lines can be shown with space prefix (optional)
        diff += `  ${line}\n`
      }
    })
  })

  return diff || ' (No visible changes)'
}

const visible = computed({
  get: () => props.modelValue,
  set: val => emit('update:modelValue', val)
})

const title = computed(() => t('workflow.approval.title'))

const dialogWidth = computed(() => {
  return isEditAction.value ? '90%' : '500px'
})

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
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;

    .label {
      font-weight: bold;
      color: var(--cs-text-color-primary);
      flex-shrink: 0;
    }

    .el-tag {
      flex-shrink: 0;
    }

    .file-path {
      font-size: var(--cs-font-size-sm);
      color: var(--cs-text-color-secondary);
      font-family: var(--cs-font-family-mono);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
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
      display: flex;
      justify-content: space-between;
      align-items: center;

      .warning-text {
        font-size: var(--cs-font-size-xs);
        color: var(--el-color-danger);
        font-style: italic;
        text-transform: none;
      }
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
      max-height: none;
      overflow: visible;
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
}
</style>

<style lang="scss">
.diff-dialog-overlay.el-overlay {
  z-index: 2000 !important;
}

.diff-dialog-overlay .el-overlay-dialog {
  overflow: hidden !important;
  position: fixed !important;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
}

.diff-dialog.el-dialog {
  position: fixed !important;
  top: var(--cs-titlebar-height, 32px) !important;
  left: 5% !important;
  width: 90% !important;
  height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  max-height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  margin: 0 !important;
  z-index: 2001 !important;
  display: flex;
  flex-direction: column;

  .el-dialog__header,
  .el-dialog__body,
  .el-dialog__footer {
    background: var(--cs-bg-color) !important;
  }

  .el-dialog__header {
    flex-shrink: 0;
    height: auto;
  }

  .el-dialog__body {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;

    .approval-content {
      flex: 1;
      display: flex;
      flex-direction: column;
      overflow: hidden;
      min-height: 0;
      max-height: 100%;

      .action-info {
        flex-shrink: 0;
      }

      .details-box {
        flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        min-height: 0;
        max-height: calc(100% - 60px);

        .details-header {
          flex-shrink: 0;
        }

        .diff-view {
          flex: 1;
          min-height: 0;
          max-height: 100%;
          overflow-y: auto;
        }
      }

      .warning-text {
        flex-shrink: 0;
      }
    }
  }

  .el-dialog__footer {
    flex-shrink: 0;
    height: auto;
  }
}
</style>
