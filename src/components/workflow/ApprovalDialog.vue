<template>
  <el-dialog v-model="visible" :title="title" :width="dialogWidth" :close-on-click-modal="false"
    :close-on-press-escape="false" :show-close="false" :class="{ 'diff-dialog': isEditAction }"
    :modal-class="isEditAction ? 'diff-dialog-overlay' : ''" custom-class="approval-dialog">
    <div class="approval-content">
      <div class="action-info">
        <span class="label">{{ $t('workflow.approval.action') }}:</span>
        <el-tag type="warning">{{ action }}</el-tag>
        <span v-if="isShellAction" class="warning-text">{{ $t('workflow.approval.warning') }}</span>
        <span v-if="isEditAction && filePath" class="file-path">{{ filePath }}</span>
      </div>
      <div class="details-box">
        <div v-if="!isShellAction" class="details-header">
          <span>{{ $t('workflow.approval.details') }}</span>
          <span class="warning-text">{{ $t('workflow.approval.warning') }}</span>
        </div>
        <div v-if="isEditAction" class="diff-view">
          <div v-if="filePath" class="diff-file-path">File: {{ filePath }}</div>
          <div class="diff-text">
            <div v-for="(line, index) in diffLines" :key="`${index}-${line.lineNumber}-${line.prefix}`"
              class="diff-line" :class="line.type">
              <span class="diff-prefix" :data-prefix="line.prefix" aria-hidden="true"></span>
              <span class="diff-line-number">{{ line.lineNumber }}</span>
              <span class="diff-separator">|</span>
              <span class="diff-content">{{ line.content }}</span>
            </div>
          </div>
        </div>
        <div v-else-if="isShellAction" class="shell-view">
          <MarkdownSimple :content="shellMarkdown" class-name="approval-markdown" />
        </div>
        <pre v-else class="details-text">{{ detailPayload.detailsText }}</pre>
      </div>
      <div class="rejection-note-box">
        <div class="note-header">{{ $t('workflow.approval.rejectionMessageLabel') }}</div>
        <el-input
          :model-value="rejectionMessage"
          type="textarea"
          :rows="isEditAction ? 2 : 3"
          resize="none"
          :placeholder="$t('workflow.approval.rejectionMessagePlaceholder')"
          @update:model-value="value => emit('update:rejectionMessage', value)"
        />
      </div>
    </div>
    <template #footer>
      <div class="dialog-footer">
        <el-button @click="onStop" :loading="loading" round type="danger">{{
          $t('workflow.pause')
        }}</el-button>
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
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import * as Diff from 'diff'
import MarkdownSimple from '@/components/workflow/MarkdownSimple.vue'

const props = defineProps({
  modelValue: Boolean,
  action: String,
  details: {
    type: [String, Object, Array],
    default: ''
  },
  rejectionMessage: {
    type: String,
    default: ''
  },
  displayType: {
    type: String,
    default: ''
  },
  loading: Boolean
})

const emit = defineEmits(['update:modelValue', 'update:rejectionMessage', 'approve', 'approveAll', 'reject', 'stop'])

const { t } = useI18n()

const normalizeDetailsPayload = (value) => {
  if (value == null || value === '') {
    return { detailsObject: null, detailsText: '' }
  }

  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value)
      const detailsObject = Array.isArray(parsed) ? (parsed[0] || null) : parsed
      return {
        detailsObject: detailsObject && typeof detailsObject === 'object' ? detailsObject : null,
        detailsText: value
      }
    } catch {
      return { detailsObject: null, detailsText: value }
    }
  }

  if (Array.isArray(value)) {
    const first = value[0] || null
    return {
      detailsObject: first && typeof first === 'object' ? first : null,
      detailsText: JSON.stringify(value, null, 2)
    }
  }

  if (typeof value === 'object') {
    return {
      detailsObject: value,
      detailsText: JSON.stringify(value, null, 2)
    }
  }

  return { detailsObject: null, detailsText: String(value) }
}

const normalizedAction = computed(() => (props.action || '').toLowerCase().trim())
const detailPayload = computed(() => normalizeDetailsPayload(props.details))
const detailsObject = computed(() => detailPayload.value.detailsObject)

const isFileChangePayload = computed(() => {
  const data = detailsObject.value
  if (!data) return false
  const hasPath = typeof data.file_path === 'string' || typeof data.path === 'string'
  const hasEditFields =
    data.old_string !== undefined ||
    data.new_string !== undefined ||
    data.content !== undefined
  return hasPath && hasEditFields
})

const isEditAction = computed(() => {
  if (props.displayType === 'diff') {
    return true
  }
  const action = normalizedAction.value
  if (action.includes('edit_file') || action.includes('write_file')) {
    return true
  }
  return isFileChangePayload.value
})

const isShellAction = computed(() => normalizedAction.value === 'bash')

const filePath = computed(() => {
  if (!isEditAction.value) return ''
  const data = detailsObject.value
  return data?.file_path || data?.path || ''
})

const diffLines = computed(() => {
  if (!isEditAction.value) return ''
  const data = detailsObject.value
  if (!data) {
    return [
      {
        prefix: ' ',
        lineNumber: '',
        content: props.details || '',
        type: 'context'
      }
    ]
  }

  const oldStr = data.old_string ?? ''
  const newStr = data.new_string ?? data.content ?? ''
  const startLine = data.start_line || 1
  return generateDiffLines(oldStr, newStr, startLine)
})

const createDiffLine = (prefix, lineNumber, content, type) => ({
  prefix,
  lineNumber,
  content,
  type
})

// Use diff library to generate proper line-by-line diff
const generateDiffLines = (oldStr, newStr, startLine = 1) => {
  if (oldStr === newStr) {
    return [createDiffLine(' ', '', '(No visible changes)', 'context')]
  }

  if (!oldStr && newStr) {
    const diff = [createDiffLine('-', '1', '(empty)', 'removed')]
    const lines = newStr.split('\n')
    if (lines[lines.length - 1] === '') {
      lines.pop()
    }
    lines.forEach((line, index) => {
      const lineNum = index + 1
      diff.push(createDiffLine('+', lineNum.toString(), line, 'added'))
    })
    return diff
  }

  const changes = Diff.diffLines(oldStr, newStr)
  const diff = []
  let currentLineOld = startLine
  let currentLineNew = startLine

  changes.forEach(change => {
    const lines = change.value.split('\n')

    // Remove last empty line if exists
    if (lines[lines.length - 1] === '') {
      lines.pop()
    }

    lines.forEach(line => {
      if (change.added) {
        diff.push(createDiffLine('+', currentLineNew.toString(), line, 'added'))
        currentLineNew++
      } else if (change.removed) {
        diff.push(createDiffLine('-', currentLineOld.toString(), line, 'removed'))
        currentLineOld++
      } else {
        diff.push(createDiffLine(' ', currentLineOld.toString(), line, 'context'))
        currentLineOld++
        currentLineNew++
      }
    })
  })

  return diff.length > 0 ? diff : [createDiffLine(' ', '', '(No visible changes)', 'context')]
}

const visible = computed({
  get: () => props.modelValue,
  set: val => emit('update:modelValue', val)
})

const title = computed(() => t('workflow.approval.title'))
const rejectionMessage = computed(() => props.rejectionMessage || '')
const shellMarkdown = computed(() => `\`\`\`bash\n${detailPayload.value.detailsText || ''}\n\`\`\``)

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

const onStop = () => {
  emit('stop')
}
</script>

<style scoped lang="scss">
.approval-content {
  .action-info {
    margin-bottom: var(--cs-space-md);
    display: flex;
    align-items: center;
    gap: var(--cs-space-sm);
    flex-wrap: wrap;

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

    .warning-text {
      font-size: var(--cs-font-size-xs);
      color: var(--el-color-danger);
      font-style: italic;
      margin-left: auto;
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
      max-height: min(28vh, 260px);
      overflow-y: auto;
    }

.shell-view {
  max-height: min(36vh, 320px);
  overflow: auto;

  :deep(pre) {
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }

  :deep(pre code.hljs) {
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
}

    .diff-view {
      max-height: none;
      overflow: visible;
      background-color: var(--cs-bg-color);
      border-radius: var(--cs-border-radius-sm);
      padding: 4px;

      .diff-file-path {
        margin-bottom: var(--cs-space-sm);
        color: var(--cs-text-color-secondary);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);
      }

      .diff-text {
        max-height: min(48vh, 520px);
        overflow: auto;
        background: var(--cs-bg-color-dark);
        border-radius: var(--cs-border-radius-sm);
        border: 1px solid var(--cs-border-color);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);

        .diff-line {
          display: grid;
          grid-template-columns: 20px minmax(44px, auto) 16px minmax(0, 1fr);
          align-items: start;
          white-space: pre;

          &.added {
            background: color-mix(in srgb, var(--el-color-success) 12%, transparent);
            color: var(--el-color-success-dark-2);
          }

          &.removed {
            background: color-mix(in srgb, var(--el-color-danger) 12%, transparent);
            color: var(--el-color-danger-dark-2);
          }

          &.context {
            color: var(--cs-text-color-primary);
          }
        }

        .diff-prefix,
        .diff-line-number,
        .diff-separator {
          user-select: none;
          opacity: 0.8;
          padding: 0 4px;
        }

        .diff-line.added .diff-prefix,
        .diff-line.removed .diff-prefix,
        .diff-line.context .diff-prefix {
          font-weight: 700;
          text-align: center;
        }

        .diff-line.added .diff-prefix::before {
          content: '+';
        }

        .diff-line.removed .diff-prefix::before {
          content: '-';
        }

        .diff-line.context .diff-prefix::before {
          content: ' ';
        }

        .diff-content {
          overflow-x: auto;
          padding-right: 8px;
        }
      }
    }
  }

  .rejection-note-box {
    margin-bottom: var(--cs-space-sm);

    .note-header {
      font-size: var(--cs-font-size-xs);
      color: var(--cs-text-color-secondary);
      margin-bottom: var(--cs-space-xs);
      text-transform: uppercase;
    }
  }
}
</style>

<style lang="scss">
.approval-dialog.el-dialog {
  max-height: calc(100vh - var(--cs-titlebar-height, 32px) - 24px);
  display: flex;
  flex-direction: column;

  .el-dialog__body {
    overflow: hidden;
  }

  .approval-content {
    max-height: calc(100vh - var(--cs-titlebar-height, 32px) - 220px);
    overflow: auto;
  }
}

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
