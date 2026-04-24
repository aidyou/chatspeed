<template>
  <div v-if="inline" class="approval-inline-panel" :class="{ 'diff-dialog': isEditAction }">
    <div class="approval-content">
      <div class="details-box" :class="{ 'plan-details-box': isPlanApproval }">
        <div v-if="isEditAction" class="diff-view">
          <div class="diff-text">
            <div
              v-for="(line, index) in diffLines"
              :key="`${index}-${line.lineNumber}-${line.prefix}`"
              class="diff-line"
              :class="line.type">
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
        <div v-else-if="isMarkdownAction" class="markdown-view">
          <MarkdownSimple :content="detailPayload.detailsText" class-name="approval-markdown" />
        </div>
        <pre v-else class="details-text">{{ detailPayload.detailsText }}</pre>
      </div>
      <div class="rejection-note-box">
        <div class="note-header">{{ $t('workflow.approval.rejectionMessageLabel') }}</div>
        <el-input
          :model-value="rejectionMessage"
          type="textarea"
          :autosize="{ minRows: 1, maxRows: 6 }"
          resize="none"
          :placeholder="$t('workflow.approval.rejectionMessagePlaceholder')"
          @update:model-value="value => emit('update:rejectionMessage', value)" />
      </div>
      <div class="dialog-footer inline-footer">
        <el-button @click="onReject" :loading="loading" round>{{ $t('common.reject') }}</el-button>
        <el-button type="primary" @click="onApprove" :loading="loading" round>{{
          $t('common.approve')
        }}</el-button>
        <el-button v-if="!isPlanApproval" type="success" @click="onApproveAll" :loading="loading" round>{{
          $t('common.approveAll')
        }}</el-button>
      </div>
    </div>
  </div>
  <el-dialog
    v-else
    v-model="visible"
    :title="action || ''"
    :width="dialogWidth"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :show-close="false"
    :class="{ 'diff-dialog': isEditAction }"
    :modal-class="isEditAction ? 'diff-dialog-overlay' : ''"
    custom-class="approval-dialog">
    <div class="approval-content">
      <div class="details-box" :class="{ 'plan-details-box': isPlanApproval }">
        <div v-if="isEditAction" class="diff-view">
          <div v-if="filePath" class="diff-file-path">File: {{ filePath }}</div>
          <div class="diff-text">
            <div
              v-for="(line, index) in diffLines"
              :key="`${index}-${line.lineNumber}-${line.prefix}`"
              class="diff-line"
              :class="line.type">
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
        <div v-else-if="isMarkdownAction" class="markdown-view">
          <MarkdownSimple :content="detailPayload.detailsText" class-name="approval-markdown" />
        </div>
        <pre v-else class="details-text">{{ detailPayload.detailsText }}</pre>
      </div>
      <div class="rejection-note-box">
        <div class="note-header">{{ $t('workflow.approval.rejectionMessageLabel') }}</div>
        <el-input
          :model-value="rejectionMessage"
          type="textarea"
          :autosize="{ minRows: 1, maxRows: 6 }"
          resize="none"
          :placeholder="$t('workflow.approval.rejectionMessagePlaceholder')"
          @update:model-value="value => emit('update:rejectionMessage', value)" />
      </div>
      <div class="dialog-footer inline-footer">
        <el-button @click="onReject" :loading="loading" round>{{ $t('common.reject') }}</el-button>
        <el-button type="primary" @click="onApprove" :loading="loading" round>{{
          $t('common.approve')
        }}</el-button>
        <el-button v-if="!isPlanApproval" type="success" @click="onApproveAll" :loading="loading" round>{{
          $t('common.approveAll')
        }}</el-button>
      </div>
    </div>
  </el-dialog>
</template>

<script setup>
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import * as Diff from 'diff'
import MarkdownSimple from '@/components/workflow/MarkdownSimple.vue'

const props = defineProps({
  modelValue: Boolean,
  inline: {
    type: Boolean,
    default: false
  },
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

const emit = defineEmits([
  'update:modelValue',
  'update:rejectionMessage',
  'approve',
  'approveAll',
  'reject'
])

useI18n()

const normalizeDetailsPayload = value => {
  if (value == null || value === '') {
    return { detailsObject: null, detailsText: '' }
  }

  if (typeof value === 'string') {
    try {
      const parsed = JSON.parse(value)
      const detailsObject = Array.isArray(parsed) ? parsed[0] || null : parsed
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
    data.old_string !== undefined || data.new_string !== undefined || data.content !== undefined
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
const isPlanApproval = computed(() => normalizedAction.value === 'submit_plan')
const isMarkdownAction = computed(() => {
  if (props.displayType === 'markdown') {
    return true
  }
  return isPlanApproval.value
})

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
  return generateDiffLines(oldStr, newStr, startLine, data)
})

const createDiffLine = (prefix, lineNumber, content, type) => ({
  prefix,
  lineNumber,
  content,
  type
})

// Use diff library to generate proper line-by-line diff
const appendContextLines = (diff, lines, startLine, type = 'context') => {
  if (!Array.isArray(lines) || !lines.length) return
  lines.forEach((line, index) => {
    diff.push(createDiffLine(' ', (startLine + index).toString(), line, type))
  })
}

const generateDiffLines = (oldStr, newStr, startLine = 1, contextData = {}) => {
  const diff = []
  appendContextLines(
    diff,
    contextData.context_before,
    contextData.context_before_start_line || Math.max(1, startLine - (contextData.context_before?.length || 0)),
    'context'
  )

  if (oldStr === newStr) {
    diff.push(createDiffLine(' ', '', '(No visible changes)', 'context'))
    appendContextLines(
      diff,
      contextData.context_after,
      contextData.context_after_start_line || startLine,
      'context'
    )
    return diff
  }

  if (!oldStr && newStr) {
    const insertionLine = startLine.toString()
    diff.push(createDiffLine('-', insertionLine, '(empty)', 'removed'))
    const lines = newStr.split('\n')
    if (lines[lines.length - 1] === '') {
      lines.pop()
    }
    lines.forEach((line, index) => {
      const lineNum = startLine + index
      diff.push(createDiffLine('+', lineNum.toString(), line, 'added'))
    })
    appendContextLines(
      diff,
      contextData.context_after,
      contextData.context_after_start_line || startLine + lines.length,
      'context'
    )
    return diff
  }

  const changes = Diff.diffLines(oldStr, newStr)
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

  if (diff.length === 0) {
    diff.push(createDiffLine(' ', '', '(No visible changes)', 'context'))
  }
  appendContextLines(
    diff,
    contextData.context_after,
    contextData.context_after_start_line || currentLineOld,
    'context'
  )
  return diff
}

const visible = computed({
  get: () => props.modelValue,
  set: val => emit('update:modelValue', val)
})

const rejectionMessage = computed(() => props.rejectionMessage || '')

const stripMarkdownFences = value => {
  if (typeof value !== 'string') return ''
  const trimmed = value.trim()
  if (!trimmed.startsWith('```')) return trimmed
  const withoutFence = trimmed.replace(/^```[a-zA-Z0-9_-]*\n?/, '').replace(/```$/, '')
  return withoutFence.trim()
}

const extractShellCommand = payload => {
  if (!payload) return ''

  const directCommand = payload.detailsObject?.command
  if (typeof directCommand === 'string' && directCommand.trim()) {
    return directCommand.trim()
  }

  const rawText = stripMarkdownFences(payload.detailsText || '')
  if (!rawText) return ''

  try {
    const parsed = JSON.parse(rawText)
    if (typeof parsed === 'string') {
      return parsed.trim()
    }
    if (parsed && typeof parsed.command === 'string') {
      return parsed.command.trim()
    }
  } catch {
    const braceStart = rawText.indexOf('{')
    const braceEnd = rawText.lastIndexOf('}')
    if (braceStart >= 0 && braceEnd > braceStart) {
      try {
        const parsed = JSON.parse(rawText.slice(braceStart, braceEnd + 1))
        if (parsed && typeof parsed.command === 'string') {
          return parsed.command.trim()
        }
      } catch {
        // Fall through to use the plain text as the command.
      }
    }
  }

  return rawText
}

const shellCommand = computed(() => extractShellCommand(detailPayload.value))
const shellMarkdown = computed(() => `\`\`\`bash\n${shellCommand.value || ''}\n\`\`\``)

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
  .details-box {
    background-color: var(--cs-bg-color-dark);
    border: 1px solid var(--cs-border-color);
    border-radius: var(--cs-border-radius-md);
    padding: var(--cs-space-sm);
    margin-bottom: var(--cs-space-md);

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

    .shell-view,
    .markdown-view {
      max-height: min(36vh, 320px);
      overflow: auto;

      :deep(pre) {
        white-space: pre-wrap;
        word-break: break-word;
        overflow-wrap: anywhere;
        margin: 0;
      }

      :deep(pre code.hljs) {
        white-space: pre-wrap;
        word-break: break-word;
        overflow-wrap: anywhere;
        background: none;
        padding: var(--cs-space-sm);
      }
    }

    &.plan-details-box {
      .markdown-view {
        max-height: none;
        overflow: visible;
      }
    }

    .diff-view {
      max-height: none;
      overflow: visible;
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
        background: var(--cs-bg-color-light);
        // border-radius: var(--cs-border-radius-sm);
        // border: 1px solid var(--cs-border-color);
        font-family: var(--cs-font-family-mono);
        font-size: var(--cs-font-size-sm);

        .diff-line {
          display: grid;
          grid-template-columns: 20px minmax(44px, auto) 16px minmax(0, 1fr);
          align-items: start;
          white-space: pre;

          &.added {
            // background: color-mix(in srgb, var(--el-color-success) 12%, transparent);
            color: var(--el-color-success-dark-2);
          }

          &.removed {
            // background: color-mix(in srgb, var(--el-color-danger) 12%, transparent);
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

.inline-footer {
  margin-top: var(--cs-space-sm);
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
  top: calc(var(--cs-titlebar-height, 32px) + 10px) !important;
  left: 5% !important;
  width: 90% !important;
  // height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  max-height: calc(100vh - var(--cs-titlebar-height, 32px) * 2) !important;
  margin: 0 !important;
  z-index: 2001 !important;
  display: flex;
  flex-direction: column;

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

      .details-box {
        // flex: 1;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        min-height: 0;
        max-height: calc(100% - 60px);

        .diff-view {
          flex: 1;
          min-height: 0;
          max-height: 100%;
          overflow-y: auto;
        }
      }
    }
  }

  .el-dialog__footer {
    flex-shrink: 0;
    height: auto;
  }
}
</style>
